use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_SCHEMA: &str = "config/MirageConfig.toml";
const DEFAULT_CONFIG: &str = "mirage.conf";
const DEFAULT_OUT_DIR: &str = "target/mirage/config";
const TUI_BODY_ROWS: usize = 17;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ConfigType {
    Bool,
    Tristate,
    String,
    Int,
    Hex,
}

impl ConfigType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Tristate => "tristate",
            Self::String => "string",
            Self::Int => "int",
            Self::Hex => "hex",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ConfigValue {
    Bool(bool),
    Tristate(char),
    String(String),
    Int(i64),
    Hex(u64),
}

impl ConfigValue {
    fn is_enabled(&self) -> bool {
        match self {
            ConfigValue::Bool(value) => *value,
            ConfigValue::Tristate(value) => *value == 'y' || *value == 'm',
            ConfigValue::String(value) => !value.is_empty(),
            ConfigValue::Int(value) => *value != 0,
            ConfigValue::Hex(value) => *value != 0,
        }
    }

    fn to_config_text(&self) -> String {
        match self {
            ConfigValue::Bool(true) => "y".to_string(),
            ConfigValue::Bool(false) => "n".to_string(),
            ConfigValue::Tristate(value) => value.to_string(),
            ConfigValue::String(value) => format!("\"{}\"", escape_config_string(value)),
            ConfigValue::Int(value) => value.to_string(),
            ConfigValue::Hex(value) => format!("0x{value:x}"),
        }
    }

    fn to_rust_literal(&self) -> String {
        match self {
            ConfigValue::Bool(value) => value.to_string(),
            ConfigValue::Tristate(value) => format!("'{}'", value),
            ConfigValue::String(value) => format!("\"{}\"", escape_config_string(value)),
            ConfigValue::Int(value) => format!("{}i64", value),
            ConfigValue::Hex(value) => format!("0x{value:x}u64"),
        }
    }

    fn short_display(&self) -> String {
        match self {
            ConfigValue::Bool(true) => "[*]".to_string(),
            ConfigValue::Bool(false) => "[ ]".to_string(),
            ConfigValue::Tristate('y') => "<Y>".to_string(),
            ConfigValue::Tristate('m') => "<M>".to_string(),
            ConfigValue::Tristate(_) => "< >".to_string(),
            ConfigValue::String(value) => format!("=\"{}\"", value),
            ConfigValue::Int(value) => format!("={value}"),
            ConfigValue::Hex(value) => format!("=0x{value:x}"),
        }
    }
}

#[derive(Clone, Debug)]
struct OptionDef {
    symbol: String,
    prompt: String,
    category: String,
    default: ConfigValue,
    help: String,
    depends_on: Vec<String>,
    selects: Vec<String>,
    visible_if: Vec<String>,
    ty: ConfigType,
    cargo_feature: Option<String>,
}

#[derive(Clone, Debug)]
struct Schema {
    options: Vec<OptionDef>,
    by_symbol: HashMap<String, usize>,
}

impl Schema {
    fn parse(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|err| format!("failed to read schema {}: {err}", path.display()))?;
        parse_schema_toml(&text)
    }

    fn get(&self, symbol: &str) -> Option<&OptionDef> {
        self.by_symbol.get(symbol).map(|idx| &self.options[*idx])
    }

    fn index(&self, symbol: &str) -> Option<usize> {
        self.by_symbol.get(symbol).copied()
    }

    fn defaults(&self) -> BTreeMap<String, ConfigValue> {
        self.options
            .iter()
            .map(|opt| (opt.symbol.clone(), opt.default.clone()))
            .collect()
    }

    fn categories(&self) -> Vec<String> {
        let mut categories = Vec::new();
        for opt in &self.options {
            if !categories.contains(&opt.category) {
                categories.push(opt.category.clone());
            }
        }
        categories
    }

    fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        for opt in &self.options {
            if !opt.symbol.starts_with("CONFIG_") {
                errors.push(format!("{}: symbol must start with CONFIG_", opt.symbol));
            }
            if opt.prompt.trim().is_empty() {
                errors.push(format!("{}: prompt is required", opt.symbol));
            }
            if opt.category.trim().is_empty() {
                errors.push(format!("{}: category is required", opt.symbol));
            }
            if opt.help.trim().is_empty() {
                errors.push(format!("{}: help is required", opt.symbol));
            }
            if !value_matches_type(&opt.ty, &opt.default) {
                errors.push(format!("{}: default value does not match type {}", opt.symbol, opt.ty.as_str()));
            }
            if let ConfigValue::Tristate(value) = &opt.default {
                if !matches!(value, 'y' | 'm' | 'n') {
                    errors.push(format!("{}: tristate default must be y, m, or n", opt.symbol));
                }
            }
            for dep in opt
                .depends_on
                .iter()
                .chain(opt.selects.iter())
                .chain(opt.visible_if.iter())
            {
                if self.get(dep).is_none() {
                    errors.push(format!("{}: references unknown symbol {dep}", opt.symbol));
                }
            }
        }
        errors.extend(self.select_cycles());
        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }

    fn select_cycles(&self) -> Vec<String> {
        fn visit(
            schema: &Schema,
            symbol: &str,
            visiting: &mut Vec<String>,
            visited: &mut HashSet<String>,
            errors: &mut Vec<String>,
        ) {
            if let Some(pos) = visiting.iter().position(|item| item == symbol) {
                let mut cycle = visiting[pos..].to_vec();
                cycle.push(symbol.to_string());
                errors.push(format!("circular select dependency: {}", cycle.join(" -> ")));
                return;
            }
            if visited.contains(symbol) {
                return;
            }
            visiting.push(symbol.to_string());
            if let Some(opt) = schema.get(symbol) {
                for selected in &opt.selects {
                    visit(schema, selected, visiting, visited, errors);
                }
            }
            visiting.pop();
            visited.insert(symbol.to_string());
        }

        let mut errors = Vec::new();
        let mut visited = HashSet::new();
        for opt in &self.options {
            visit(self, &opt.symbol, &mut Vec::new(), &mut visited, &mut errors);
        }
        errors.sort();
        errors.dedup();
        errors
    }
}

#[derive(Clone, Debug)]
struct ParsedConfig {
    values: BTreeMap<String, ConfigValue>,
}

#[derive(Default)]
struct Args {
    menu: bool,
    defconfig: bool,
    oldconfig: bool,
    savedefconfig: bool,
    list: bool,
    check: bool,
    config: PathBuf,
    output: Option<PathBuf>,
    schema: PathBuf,
    generate: bool,
    out_dir: PathBuf,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(env::args().skip(1))?;
    let schema = Schema::parse(&args.schema)?;
    if let Err(errors) = schema.validate() {
        return Err(errors.join("\n"));
    }

    let command_count = [args.menu, args.defconfig, args.oldconfig, args.savedefconfig, args.list, args.check]
        .iter()
        .filter(|enabled| **enabled)
        .count();
    if command_count != 1 {
        return Err("select exactly one of --menu, --defconfig, --oldconfig, --savedefconfig, --list, or --check".to_string());
    }

    if args.list {
        list_config(&schema);
        return Ok(());
    }

    if args.defconfig {
        let values = resolve_selects(&schema, schema.defaults())?;
        validate_values(&schema, &values, false)?;
        let output = args.output.as_deref().unwrap_or(&args.config);
        write_config(output, &schema, &values, false)?;
        if args.generate { generate_artifacts(&schema, &values, &args.out_dir)?; }
        return Ok(());
    }

    if args.check {
        let parsed = read_config(&args.config, &schema)?;
        validate_values(&schema, &parsed.values, true)?;
        let values = resolve_selects(&schema, parsed.values.clone())?;
        validate_values(&schema, &values, true)?;
        if args.generate { generate_artifacts(&schema, &values, &args.out_dir)?; }
        return Ok(());
    }

    if args.oldconfig {
        let mut values = schema.defaults();
        if let Some(parsed) = read_config_if_present(&args.config, &schema)? {
            for (symbol, value) in parsed.values { values.insert(symbol, value); }
        }
        let values = resolve_selects(&schema, values)?;
        validate_values(&schema, &values, false)?;
        let output = args.output.as_deref().unwrap_or(&args.config);
        write_config(output, &schema, &values, false)?;
        if args.generate { generate_artifacts(&schema, &values, &args.out_dir)?; }
        return Ok(());
    }

    if args.savedefconfig {
        let parsed = read_config(&args.config, &schema)?;
        let values = resolve_selects(&schema, parsed.values)?;
        validate_values(&schema, &values, true)?;
        let output = args.output.unwrap_or_else(|| PathBuf::from("mirage.defconfig"));
        write_config(&output, &schema, &values, true)?;
        if args.generate { generate_artifacts(&schema, &values, &args.out_dir)?; }
        return Ok(());
    }

    if args.menu {
        let mut values = schema.defaults();
        if let Some(parsed) = read_config_if_present(&args.config, &schema)? {
            for (symbol, value) in parsed.values { values.insert(symbol, value); }
        }
        match menu(&schema, &mut values)? {
            MenuOutcome::Save => {
                let values = resolve_selects(&schema, values)?;
                validate_values(&schema, &values, false)?;
                let output = args.output.as_deref().unwrap_or(&args.config);
                write_config(output, &schema, &values, false)?;
                if args.generate { generate_artifacts(&schema, &values, &args.out_dir)?; }
            }
            MenuOutcome::Discard => {
                println!("configuration unchanged");
            }
        }
    }

    Ok(())
}

fn parse_args<I>(iter: I) -> Result<Args, String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = Args {
        config: PathBuf::from(DEFAULT_CONFIG),
        schema: PathBuf::from(DEFAULT_SCHEMA),
        out_dir: PathBuf::from(DEFAULT_OUT_DIR),
        ..Args::default()
    };
    let mut iter = iter.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--menu" | "--menuconfig" => args.menu = true,
            "--defconfig" => args.defconfig = true,
            "--oldconfig" | "--olddefconfig" => args.oldconfig = true,
            "--savedefconfig" => args.savedefconfig = true,
            "--list" => args.list = true,
            "--check" => args.check = true,
            "--generate" => args.generate = true,
            "--config" => args.config = PathBuf::from(next_arg(&mut iter, "--config")?),
            "--output" => args.output = Some(PathBuf::from(next_arg(&mut iter, "--output")?)),
            "--schema" => args.schema = PathBuf::from(next_arg(&mut iter, "--schema")?),
            "--out-dir" => args.out_dir = PathBuf::from(next_arg(&mut iter, "--out-dir")?),
            "--help" | "-h" => { print_help(); std::process::exit(0); }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(args)
}

fn next_arg(iter: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    iter.next().ok_or_else(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!("Mirage configuration utility");
    println!("  --menu, --menuconfig   ncurses-style expandable/selectable menu");
    println!("  --defconfig            write defaults");
    println!("  --oldconfig            merge existing config with schema defaults");
    println!("  --savedefconfig        write only values differing from defaults");
    println!("  --list                 list schema options grouped by menu");
    println!("  --check                validate config");
    println!("  --config <file>        input/output config file (default mirage.conf)");
    println!("  --output <file>        output file for writing commands");
    println!("  --schema <file>        schema file (default config/MirageConfig.toml)");
    println!("  --out-dir <dir>        generated artifact directory");
    println!("  --generate             generate target/mirage/config artifacts");
}

#[derive(Clone, Debug)]
enum TomlValue {
    Bool(bool),
    String(String),
    Array(Vec<String>),
    Int(i64),
    Hex(u64),
}

fn parse_schema_toml(text: &str) -> Result<Schema, String> {
    let mut tables: Vec<BTreeMap<String, TomlValue>> = Vec::new();
    let mut current: Option<BTreeMap<String, TomlValue>> = None;

    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let stripped = strip_toml_comment(raw_line);
        let line = stripped.trim();
        if line.is_empty() { continue; }
        if line == "[[options]]" {
            if let Some(table) = current.take() { tables.push(table); }
            current = Some(BTreeMap::new());
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("schema line {line_no}: expected key = value"))?;
        let key = key.trim().to_string();
        let value = parse_toml_value(value.trim())
            .map_err(|err| format!("schema line {line_no}: {err}"))?;
        let table = current
            .as_mut()
            .ok_or_else(|| format!("schema line {line_no}: key outside [[options]]"))?;
        if table.insert(key.clone(), value).is_some() {
            return Err(format!("schema line {line_no}: duplicate key {key}"));
        }
    }
    if let Some(table) = current.take() { tables.push(table); }

    let mut options = Vec::new();
    let mut by_symbol = HashMap::new();
    for (idx, table) in tables.into_iter().enumerate() {
        let symbol = required_string(&table, "symbol", idx)?;
        if by_symbol.contains_key(&symbol) {
            return Err(format!("schema has duplicate symbol {symbol}"));
        }
        let ty = match required_string(&table, "type", idx)?.as_str() {
            "bool" => ConfigType::Bool,
            "tristate" => ConfigType::Tristate,
            "string" => ConfigType::String,
            "int" => ConfigType::Int,
            "hex" => ConfigType::Hex,
            other => return Err(format!("{symbol}: unsupported type '{other}'")),
        };
        let default = parse_default(&ty, table.get("default").ok_or_else(|| format!("{symbol}: missing default"))?)?;
        let option = OptionDef {
            prompt: required_string(&table, "prompt", idx)?,
            category: required_string(&table, "category", idx)?,
            help: required_string(&table, "help", idx)?,
            depends_on: optional_array(&table, "depends_on")?,
            selects: optional_array(&table, "selects")?,
            visible_if: optional_array(&table, "visible_if")?,
            cargo_feature: optional_string(&table, "cargo_feature")?,
            symbol: symbol.clone(),
            default,
            ty,
        };
        by_symbol.insert(symbol, options.len());
        options.push(option);
    }

    Ok(Schema { options, by_symbol })
}

fn strip_toml_comment(line: &str) -> String {
    let mut out = String::new();
    let mut in_string = false;
    let mut escape = false;
    for ch in line.chars() {
        if escape {
            out.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' && in_string {
            out.push(ch);
            escape = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            out.push(ch);
            continue;
        }
        if ch == '#' && !in_string {
            break;
        }
        out.push(ch);
    }
    out
}

fn parse_toml_value(raw: &str) -> Result<TomlValue, String> {
    if raw == "true" { return Ok(TomlValue::Bool(true)); }
    if raw == "false" { return Ok(TomlValue::Bool(false)); }
    if raw.starts_with('"') {
        return Ok(TomlValue::String(parse_quoted(raw)?));
    }
    if raw.starts_with('[') {
        return parse_array(raw).map(TomlValue::Array);
    }
    if let Some(hex) = raw.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16)
            .map(TomlValue::Hex)
            .map_err(|err| format!("invalid hex literal: {err}"));
    }
    raw.parse::<i64>()
        .map(TomlValue::Int)
        .map_err(|err| format!("unsupported TOML value '{raw}': {err}"))
}

fn parse_quoted(raw: &str) -> Result<String, String> {
    let bytes = raw.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || *bytes.last().unwrap() != b'"' {
        return Err(format!("unterminated string literal {raw}"));
    }
    let mut out = String::new();
    let mut escape = false;
    for ch in raw[1..raw.len() - 1].chars() {
        if escape {
            match ch {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                other => out.push(other),
            }
            escape = false;
        } else if ch == '\\' {
            escape = true;
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

fn parse_array(raw: &str) -> Result<Vec<String>, String> {
    if !raw.ends_with(']') { return Err(format!("unterminated array {raw}")); }
    let inner = raw[1..raw.len() - 1].trim();
    if inner.is_empty() { return Ok(Vec::new()); }
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escape = false;
    for ch in inner.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' && in_string {
            current.push(ch);
            escape = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            current.push(ch);
            continue;
        }
        if ch == ',' && !in_string {
            let item = current.trim();
            if !item.is_empty() { result.push(parse_quoted(item)?); }
            current.clear();
        } else {
            current.push(ch);
        }
    }
    let item = current.trim();
    if !item.is_empty() { result.push(parse_quoted(item)?); }
    Ok(result)
}

fn required_string(table: &BTreeMap<String, TomlValue>, key: &str, idx: usize) -> Result<String, String> {
    match table.get(key) {
        Some(TomlValue::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("option #{idx}: {key} must be a string")),
        None => Err(format!("option #{idx}: missing {key}")),
    }
}

fn optional_string(table: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<String>, String> {
    match table.get(key) {
        Some(TomlValue::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{key} must be a string")),
        None => Ok(None),
    }
}

fn optional_array(table: &BTreeMap<String, TomlValue>, key: &str) -> Result<Vec<String>, String> {
    match table.get(key) {
        Some(TomlValue::Array(value)) => Ok(value.clone()),
        Some(_) => Err(format!("{key} must be an array")),
        None => Ok(Vec::new()),
    }
}

fn parse_default(ty: &ConfigType, value: &TomlValue) -> Result<ConfigValue, String> {
    match (ty, value) {
        (ConfigType::Bool, TomlValue::Bool(v)) => Ok(ConfigValue::Bool(*v)),
        (ConfigType::Tristate, TomlValue::String(v)) if matches!(v.as_str(), "y" | "m" | "n") => {
            Ok(ConfigValue::Tristate(v.chars().next().unwrap()))
        }
        (ConfigType::String, TomlValue::String(v)) => Ok(ConfigValue::String(v.clone())),
        (ConfigType::Int, TomlValue::Int(v)) => Ok(ConfigValue::Int(*v)),
        (ConfigType::Hex, TomlValue::Hex(v)) => Ok(ConfigValue::Hex(*v)),
        (ConfigType::Hex, TomlValue::Int(v)) if *v >= 0 => Ok(ConfigValue::Hex(*v as u64)),
        _ => Err("default does not match option type".to_string()),
    }
}

fn value_matches_type(ty: &ConfigType, value: &ConfigValue) -> bool {
    matches!(
        (ty, value),
        (ConfigType::Bool, ConfigValue::Bool(_))
            | (ConfigType::Tristate, ConfigValue::Tristate(_))
            | (ConfigType::String, ConfigValue::String(_))
            | (ConfigType::Int, ConfigValue::Int(_))
            | (ConfigType::Hex, ConfigValue::Hex(_))
    )
}

fn read_config_if_present(path: &Path, schema: &Schema) -> Result<Option<ParsedConfig>, String> {
    if path.exists() { read_config(path, schema).map(Some) } else { Ok(None) }
}

fn read_config(path: &Path, schema: &Schema) -> Result<ParsedConfig, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read config {}: {err}", path.display()))?;
    let mut values = schema.defaults();
    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() { continue; }
        if let Some(rest) = line.strip_prefix("# ") {
            if let Some(symbol) = rest.strip_suffix(" is not set") {
                if schema.get(symbol).is_some() {
                    values.insert(symbol.to_string(), ConfigValue::Bool(false));
                }
                continue;
            }
        }
        if line.starts_with('#') { continue; }
        let (symbol, raw_value) = line
            .split_once('=')
            .ok_or_else(|| format!("config line {line_no}: expected SYMBOL=value"))?;
        let symbol = symbol.trim();
        let option = schema
            .get(symbol)
            .ok_or_else(|| format!("config line {line_no}: unknown symbol {symbol}"))?;
        let value = parse_config_value(&option.ty, raw_value.trim())
            .map_err(|err| format!("config line {line_no}: {symbol}: {err}"))?;
        values.insert(symbol.to_string(), value);
    }
    Ok(ParsedConfig { values })
}

fn parse_config_value(ty: &ConfigType, raw: &str) -> Result<ConfigValue, String> {
    match ty {
        ConfigType::Bool => match raw {
            "y" | "Y" | "1" | "true" => Ok(ConfigValue::Bool(true)),
            "n" | "N" | "0" | "false" => Ok(ConfigValue::Bool(false)),
            _ => Err("expected y or n".to_string()),
        },
        ConfigType::Tristate => match raw {
            "y" | "Y" => Ok(ConfigValue::Tristate('y')),
            "m" | "M" => Ok(ConfigValue::Tristate('m')),
            "n" | "N" => Ok(ConfigValue::Tristate('n')),
            _ => Err("expected y, m, or n".to_string()),
        },
        ConfigType::String => parse_quoted(raw).map(ConfigValue::String),
        ConfigType::Int => raw.parse::<i64>().map(ConfigValue::Int).map_err(|err| err.to_string()),
        ConfigType::Hex => {
            if let Some(hex) = raw.strip_prefix("0x") {
                u64::from_str_radix(hex, 16).map(ConfigValue::Hex).map_err(|err| err.to_string())
            } else {
                raw.parse::<u64>().map(ConfigValue::Hex).map_err(|err| err.to_string())
            }
        }
    }
}

fn write_config(path: &Path, schema: &Schema, values: &BTreeMap<String, ConfigValue>, minimal: bool) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let mut out = String::new();
    out.push_str("#\n# Automatically generated file; DO NOT EDIT.\n# Mirage Kernel Configuration\n#\n\n");
    let mut last_category = String::new();
    for opt in &schema.options {
        let value = values.get(&opt.symbol).unwrap_or(&opt.default);
        if minimal && *value == opt.default { continue; }
        if opt.category != last_category {
            if !last_category.is_empty() { out.push('\n'); }
            out.push_str("#\n# ");
            out.push_str(&opt.category);
            out.push_str("\n#\n");
            last_category = opt.category.clone();
        }
        match value {
            ConfigValue::Bool(false) => {
                out.push_str("# ");
                out.push_str(&opt.symbol);
                out.push_str(" is not set\n");
            }
            _ => {
                out.push_str(&opt.symbol);
                out.push('=');
                out.push_str(&value.to_config_text());
                out.push('\n');
            }
        }
    }
    fs::write(path, out).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn resolve_selects(schema: &Schema, mut values: BTreeMap<String, ConfigValue>) -> Result<BTreeMap<String, ConfigValue>, String> {
    let mut changed = true;
    let mut passes = 0usize;
    while changed {
        changed = false;
        passes += 1;
        if passes > schema.options.len().saturating_add(1) {
            return Err("select resolution did not converge".to_string());
        }
        for opt in &schema.options {
            if value_enabled(values.get(&opt.symbol)) {
                for selected in &opt.selects {
                    if let Some(target) = schema.get(selected) {
                        let next = match target.ty {
                            ConfigType::Bool => ConfigValue::Bool(true),
                            ConfigType::Tristate => ConfigValue::Tristate('y'),
                            _ => target.default.clone(),
                        };
                        if values.get(selected) != Some(&next) {
                            values.insert(selected.clone(), next);
                            changed = true;
                        }
                    }
                }
            }
        }
    }
    Ok(values)
}

fn validate_values(schema: &Schema, values: &BTreeMap<String, ConfigValue>, strict: bool) -> Result<(), String> {
    let mut errors = Vec::new();
    if strict {
        for symbol in values.keys() {
            if schema.get(symbol).is_none() {
                errors.push(format!("unknown config symbol {symbol}"));
            }
        }
    }
    for opt in &schema.options {
        let value = values.get(&opt.symbol).unwrap_or(&opt.default);
        if !value_matches_type(&opt.ty, value) {
            errors.push(format!("{}: value does not match type {}", opt.symbol, opt.ty.as_str()));
            continue;
        }
        if value.is_enabled() {
            for dep in &opt.depends_on {
                if !value_enabled(values.get(dep)) {
                    errors.push(format!("{} depends on {dep}", opt.symbol));
                }
            }
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

fn value_enabled(value: Option<&ConfigValue>) -> bool {
    value.map(ConfigValue::is_enabled).unwrap_or(false)
}

fn option_visible(opt: &OptionDef, values: &BTreeMap<String, ConfigValue>) -> bool {
    opt.visible_if.iter().all(|sym| value_enabled(values.get(sym)))
}

fn deps_satisfied(opt: &OptionDef, values: &BTreeMap<String, ConfigValue>) -> bool {
    opt.depends_on.iter().all(|sym| value_enabled(values.get(sym)))
}

fn list_config(schema: &Schema) {
    let mut current = String::new();
    for opt in &schema.options {
        if opt.category != current {
            current = opt.category.clone();
            println!("\n{}", current);
            println!("{}", "-".repeat(current.len()));
        }
        println!("{} ({})", opt.symbol, opt.ty.as_str());
        println!("  prompt:  {}", opt.prompt);
        println!("  default: {}", opt.default.to_config_text());
        if !opt.depends_on.is_empty() { println!("  depends: {}", opt.depends_on.join(", ")); }
        if !opt.selects.is_empty() { println!("  selects: {}", opt.selects.join(", ")); }
        if let Some(feature) = &opt.cargo_feature { println!("  cargo:   {feature}"); }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MenuOutcome { Save, Discard }

fn menu(schema: &Schema, values: &mut BTreeMap<String, ConfigValue>) -> Result<MenuOutcome, String> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        run_tui(schema, values)
    } else {
        run_line_menu(schema, values)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum MenuRow {
    Back,
    Category(String),
    Option(usize),
}

#[derive(Clone, Debug)]
struct MenuState {
    categories: Vec<String>,
    expanded: BTreeSet<String>,
    current_menu: Option<String>,
    cursor: usize,
    scroll: usize,
    show_help: bool,
    search: String,
    dirty: bool,
}

impl MenuState {
    fn new(schema: &Schema) -> Self {
        let categories = schema.categories();
        let mut expanded = BTreeSet::new();
        if let Some(first) = categories.first() {
            expanded.insert(first.clone());
        }
        Self {
            categories,
            expanded,
            current_menu: None,
            cursor: 0,
            scroll: 0,
            show_help: true,
            search: String::new(),
            dirty: false,
        }
    }
}

struct TerminalGuard { original: String }

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        let output = Command::new("stty").arg("-g").output()?;
        let original = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Do not use `stty raw` here. Raw mode disables output post-processing
        // on many terminals, which means `\n` no longer returns to column 0 and
        // the menu renders diagonally across the screen. This is closer to
        // cbreak mode: single-key input without breaking normal terminal output.
        let _ = Command::new("stty")
            .args(["-echo", "-icanon", "min", "0", "time", "1"])
            .status();

        // Alternate screen keeps menuconfig from polluting the scrollback.
        print!("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        io::stdout().flush()?;
        Ok(Self { original })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.original.is_empty() {
            let _ = Command::new("stty").arg("sane").status();
        } else {
            let _ = Command::new("stty").arg(&self.original).status();
        }
        print!("\x1b[?25h\x1b[0m\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Backspace,
    Enter,
    Esc,
    Char(char),
}

fn read_key() -> io::Result<Key> {
    let mut stdin = io::stdin();
    let mut buf = [0u8; 8];
    loop {
        let n = stdin.read(&mut buf[..1])?;
        if n == 0 { continue; }
        break;
    }
    match buf[0] {
        b'\r' | b'\n' => Ok(Key::Enter),
        0x7f | 0x08 => Ok(Key::Backspace),
        0x1b => {
            let n = stdin.read(&mut buf[1..])?;
            if n >= 2 && buf[1] == b'[' {
                Ok(match buf[2] {
                    b'A' => Key::Up,
                    b'B' => Key::Down,
                    b'C' => Key::Right,
                    b'D' => Key::Left,
                    b'H' => Key::Home,
                    b'F' => Key::End,
                    b'5' => Key::PageUp,
                    b'6' => Key::PageDown,
                    _ => Key::Esc,
                })
            } else {
                Ok(Key::Esc)
            }
        }
        byte => Ok(Key::Char(byte as char)),
    }
}

fn run_tui(schema: &Schema, values: &mut BTreeMap<String, ConfigValue>) -> Result<MenuOutcome, String> {
    let _guard = TerminalGuard::enter().map_err(|err| format!("failed to enter terminal menu mode: {err}"))?;
    let mut state = MenuState::new(schema);

    loop {
        let rows = visible_rows(schema, values, &state);
        clamp_cursor(&mut state, rows.len());
        render(schema, values, &state, &rows).map_err(|err| err.to_string())?;

        match read_key().map_err(|err| err.to_string())? {
            Key::Up | Key::Char('k') => move_cursor(&mut state, rows.len(), -1),
            Key::Down | Key::Char('j') => move_cursor(&mut state, rows.len(), 1),
            Key::PageUp => move_cursor(&mut state, rows.len(), -(TUI_BODY_ROWS as isize)),
            Key::PageDown => move_cursor(&mut state, rows.len(), TUI_BODY_ROWS as isize),
            Key::Home => {
                state.cursor = 0;
                state.scroll = 0;
            }
            Key::End => {
                state.cursor = rows.len().saturating_sub(1);
            }
            Key::Left => {
                if state.current_menu.is_some() {
                    state.current_menu = None;
                    state.cursor = 0;
                    state.scroll = 0;
                }
            }
            Key::Right | Key::Enter => {
                let cursor = state.cursor;
                menu_select(schema, values, &mut state, rows.get(cursor))?;
            }
            Key::Char(' ') | Key::Char('y') | Key::Char('n') | Key::Char('m') => {
                if let Some(MenuRow::Option(idx)) = rows.get(state.cursor) {
                    toggle_option(schema, values, *idx, &mut state)?;
                }
            }
            Key::Char('?') | Key::Char('h') => state.show_help = !state.show_help,
            Key::Char('/') => {
                state.search = prompt_raw("Search", &state.search).map_err(|err| err.to_string())?;
                state.current_menu = None;
                state.cursor = 0;
                state.scroll = 0;
            }
            Key::Char('c') => {
                state.search.clear();
                state.cursor = 0;
                state.scroll = 0;
            }
            Key::Char('s') => return Ok(MenuOutcome::Save),
            Key::Char('q') | Key::Esc => {
                if state.current_menu.is_some() {
                    state.current_menu = None;
                    state.cursor = 0;
                    state.scroll = 0;
                    continue;
                }

                if state.dirty {
                    let answer = prompt_key("Save changed configuration? [Y/n]").map_err(|err| err.to_string())?;
                    if matches!(answer, Key::Char('n') | Key::Char('N')) {
                        return Ok(MenuOutcome::Discard);
                    }
                    return Ok(MenuOutcome::Save);
                }
                return Ok(MenuOutcome::Discard);
            }
            _ => {}
        }
    }
}
fn visible_rows(schema: &Schema, values: &BTreeMap<String, ConfigValue>, state: &MenuState) -> Vec<MenuRow> {
    let mut rows = Vec::new();
    let search = state.search.to_lowercase();

    if !search.is_empty() {
        for (idx, opt) in schema.options.iter().enumerate() {
            if !option_visible(opt, values) {
                continue;
            }
            let hay = format!("{} {} {} {}", opt.symbol, opt.prompt, opt.category, opt.help).to_lowercase();
            if hay.contains(&search) {
                rows.push(MenuRow::Option(idx));
            }
        }
        return rows;
    }

    if let Some(category) = &state.current_menu {
        rows.push(MenuRow::Back);
        for (idx, opt) in schema.options.iter().enumerate() {
            if &opt.category == category && option_visible(opt, values) {
                rows.push(MenuRow::Option(idx));
            }
        }
        return rows;
    }

    for category in &state.categories {
        if schema
            .options
            .iter()
            .any(|opt| &opt.category == category && option_visible(opt, values))
        {
            rows.push(MenuRow::Category(category.clone()));
        }
    }

    rows
}
fn render(schema: &Schema, values: &BTreeMap<String, ConfigValue>, state: &MenuState, rows: &[MenuRow]) -> io::Result<()> {
    let mut out = io::stdout();
    let (term_rows, term_cols) = terminal_size();
    let width = bounded(term_cols.saturating_sub(4), 76, 132);
    let height = bounded(term_rows.saturating_sub(4), 22, 44);
    let body_rows = height.saturating_sub(13);
    let left = term_cols.saturating_sub(width) / 2 + 1;
    let top = 2usize;

    write!(out, "\x1b[H\x1b[2J")?;
    write!(out, "\x1b[36m.config - Mirage Kernel Configuration\x1b[0m")?;

    let title = if let Some(menu) = &state.current_menu {
        menu.as_str()
    } else if state.search.is_empty() {
        "Mirage Kernel Configuration"
    } else {
        "Search results"
    };

    draw_box(&mut out, top, left, width, height, title)?;

    let inner_left = left + 2;
    let mut row = top + 2;

    goto(&mut out, row, inner_left)?;
    write!(out, "Arrow keys navigate the menu.  <Enter> selects submenus --->.  Highlighted letters are hotkeys.")?;
    row += 1;
    goto(&mut out, row, inner_left)?;
    write!(out, "Press <Y> includes, <N> excludes, <M> modularizes.  <Esc><Esc> exits, <?> Help, </> Search.")?;
    row += 2;

    hline(&mut out, row, inner_left, width.saturating_sub(4))?;
    row += 1;

    let mut scroll = state.scroll;
    if state.cursor < scroll {
        scroll = state.cursor;
    }
    if state.cursor >= scroll + body_rows {
        scroll = state.cursor + 1 - body_rows;
    }
    let end = usize::min(scroll + body_rows, rows.len());

    let menu_width = width.saturating_sub(14);
    let menu_left = left + (width.saturating_sub(menu_width) / 2);

    for row_index in scroll..end {
        goto(&mut out, row, menu_left)?;
        let selected = row_index == state.cursor;
        let line = render_linux_row(schema, values, state, &rows[row_index], menu_width);

        if selected {
            write!(out, "\x1b[44m\x1b[37m")?;
        }
        write!(out, "{}", fit_text(&line, menu_width))?;
        let used = line.chars().count().min(menu_width);
        if used < menu_width {
            write!(out, "{}", " ".repeat(menu_width - used))?;
        }
        if selected {
            write!(out, "\x1b[0m")?;
        }
        row += 1;
    }

    let separator_row = top + height.saturating_sub(5);
    hline(&mut out, separator_row, inner_left, width.saturating_sub(4))?;

    let help_row = separator_row + 1;
    clear_line_region(&mut out, help_row, inner_left, width.saturating_sub(4))?;
    goto(&mut out, help_row, inner_left)?;
    if state.show_help {
        write!(out, "{}", fit_text(&selected_help(schema, rows.get(state.cursor)), width.saturating_sub(4)))?;
    } else {
        write!(out, "Help hidden; press ? to show.")?;
    }

    let status_row = top + height.saturating_sub(2);
    goto(&mut out, status_row, left + width.saturating_sub(64) / 2)?;
    write!(out, "\x1b[44m<Select>\x1b[0m    < Exit >    < Help >    < Save >    < Search >")?;

    if !state.search.is_empty() {
        goto(&mut out, top + 1, left + 3)?;
        write!(out, "\x1b[36m/ {}\x1b[0m", fit_text(&state.search, width.saturating_sub(8)))?;
    }

    out.flush()
}

fn render_linux_row(
    schema: &Schema,
    values: &BTreeMap<String, ConfigValue>,
    state: &MenuState,
    row: &MenuRow,
    width: usize,
) -> String {
    match row {
        MenuRow::Back => "<-- Back".to_string(),
        MenuRow::Category(category) => {
            let total = schema
                .options
                .iter()
                .filter(|opt| &opt.category == category)
                .count();
            let enabled = schema
                .options
                .iter()
                .filter(|opt| &opt.category == category && value_enabled(values.get(&opt.symbol)))
                .count();
            let left = format!("    {category}  --->");
            let stats = format!("{enabled}/{total}");
            right_status(left, stats, width)
        }
        MenuRow::Option(idx) => {
            let opt = &schema.options[*idx];
            let value = values.get(&opt.symbol).unwrap_or(&opt.default);
            let marker = linux_marker(value, &opt.ty);
            let blocked = !deps_satisfied(opt, values);
            let left = if blocked {
                format!("    -{}- {}", marker.trim_matches(&['[', ']', '<', '>'][..]), opt.prompt)
            } else if matches!(opt.ty, ConfigType::String | ConfigType::Int | ConfigType::Hex) {
                format!("    ({}) {}", value.to_config_text(), opt.prompt)
            } else {
                format!("    {marker} {}", opt.prompt)
            };

            if state.search.is_empty() {
                left
            } else {
                right_status(left, opt.category.clone(), width)
            }
        }
    }
}

fn linux_marker(value: &ConfigValue, ty: &ConfigType) -> String {
    match (ty, value) {
        (ConfigType::Bool, ConfigValue::Bool(true)) => "[*]".to_string(),
        (ConfigType::Bool, ConfigValue::Bool(false)) => "[ ]".to_string(),
        (ConfigType::Tristate, ConfigValue::Tristate('y')) => "<*>".to_string(),
        (ConfigType::Tristate, ConfigValue::Tristate('m')) => "<M>".to_string(),
        (ConfigType::Tristate, _) => "< >".to_string(),
        _ => "( )".to_string(),
    }
}

fn selected_help(schema: &Schema, row: Option<&MenuRow>) -> String {
    match row {
        Some(MenuRow::Option(idx)) => {
            let opt = &schema.options[*idx];
            let mut help = opt.help.clone();
            if !opt.depends_on.is_empty() {
                help.push_str("  Depends on: ");
                help.push_str(&opt.depends_on.join(", "));
            }
            if !opt.selects.is_empty() {
                help.push_str("  Selects: ");
                help.push_str(&opt.selects.join(", "));
            }
            if let Some(feature) = &opt.cargo_feature {
                help.push_str("  Cargo feature: ");
                help.push_str(feature);
            }
            help
        }
        Some(MenuRow::Category(category)) => format!("{category} submenu. Press Enter to open."),
        Some(MenuRow::Back) => "Return to the previous menu.".to_string(),
        None => String::new(),
    }
}

fn right_status(left: String, status: String, width: usize) -> String {
    let left_len = left.chars().count();
    let status_len = status.chars().count();
    if left_len + status_len + 1 >= width {
        return left;
    }
    format!("{left}{}{}", " ".repeat(width - left_len - status_len), status)
}

fn draw_box(out: &mut impl Write, top: usize, left: usize, width: usize, height: usize, title: &str) -> io::Result<()> {
    goto(out, top, left)?;
    write!(out, "┌{}┐", "─".repeat(width.saturating_sub(2)))?;

    let title = fit_text(title, width.saturating_sub(8));
    goto(out, top, left + (width.saturating_sub(title.chars().count()) / 2))?;
    write!(out, "\x1b[1m{title}\x1b[0m")?;

    for row in (top + 1)..(top + height.saturating_sub(1)) {
        goto(out, row, left)?;
        write!(out, "│{}│", " ".repeat(width.saturating_sub(2)))?;
    }

    goto(out, top + height.saturating_sub(1), left)?;
    write!(out, "└{}┘", "─".repeat(width.saturating_sub(2)))?;
    Ok(())
}

fn hline(out: &mut impl Write, row: usize, col: usize, width: usize) -> io::Result<()> {
    goto(out, row, col)?;
    write!(out, "{}", "─".repeat(width))
}

fn clear_line_region(out: &mut impl Write, row: usize, col: usize, width: usize) -> io::Result<()> {
    goto(out, row, col)?;
    write!(out, "{}", " ".repeat(width))
}

fn goto(out: &mut impl Write, row: usize, col: usize) -> io::Result<()> {
    write!(out, "\x1b[{row};{col}H")
}

fn terminal_size() -> (usize, usize) {
    if let Ok(output) = Command::new("stty").arg("size").output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut parts = text.split_whitespace();
            if let (Some(rows), Some(cols)) = (parts.next(), parts.next()) {
                if let (Ok(rows), Ok(cols)) = (rows.parse::<usize>(), cols.parse::<usize>()) {
                    return (rows, cols);
                }
            }
        }
    }

    let rows = env::var("LINES").ok().and_then(|v| v.parse().ok()).unwrap_or(32);
    let cols = env::var("COLUMNS").ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    (rows, cols)
}

fn bounded(value: usize, min: usize, max: usize) -> usize {
    value.max(min).min(max)
}

fn fit_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width <= 3 {
        return text.chars().take(width).collect();
    }
    let mut out: String = text.chars().take(width - 3).collect();
    out.push_str("...");
    out
}

fn clamp_cursor(state: &mut MenuState, len: usize) {
    if len == 0 {
        state.cursor = 0;
        state.scroll = 0;
    } else if state.cursor >= len {
        state.cursor = len - 1;
    }
}

fn move_cursor(state: &mut MenuState, len: usize, delta: isize) {
    if len == 0 { return; }
    let next = (state.cursor as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
    state.cursor = next;
}

fn menu_select(schema: &Schema, values: &mut BTreeMap<String, ConfigValue>, state: &mut MenuState, row: Option<&MenuRow>) -> Result<(), String> {
    match row {
        Some(MenuRow::Back) => {
            state.current_menu = None;
            state.cursor = 0;
            state.scroll = 0;
        }
        Some(MenuRow::Category(category)) => {
            state.current_menu = Some(category.clone());
            state.cursor = 0;
            state.scroll = 0;
        }
        Some(MenuRow::Option(idx)) => toggle_option(schema, values, *idx, state)?,
        None => {}
    }
    Ok(())
}

fn toggle_option(schema: &Schema, values: &mut BTreeMap<String, ConfigValue>, idx: usize, state: &mut MenuState) -> Result<(), String> {
    let opt = &schema.options[idx];
    if !deps_satisfied(opt, values) {
        return Ok(());
    }
    let next = match values.get(&opt.symbol).unwrap_or(&opt.default).clone() {
        ConfigValue::Bool(value) => ConfigValue::Bool(!value),
        ConfigValue::Tristate('n') => ConfigValue::Tristate('m'),
        ConfigValue::Tristate('m') => ConfigValue::Tristate('y'),
        ConfigValue::Tristate(_) => ConfigValue::Tristate('n'),
        current @ ConfigValue::String(_) | current @ ConfigValue::Int(_) | current @ ConfigValue::Hex(_) => {
            let raw = prompt_raw(&opt.prompt, &current.to_config_text()).map_err(|err| err.to_string())?;
            parse_config_value(&opt.ty, raw.trim()).unwrap_or(current)
        }
    };
    values.insert(opt.symbol.clone(), next.clone());
    if next.is_enabled() {
        apply_selects_from(schema, values, &opt.symbol)?;
    }
    state.dirty = true;
    Ok(())
}

fn apply_selects_from(schema: &Schema, values: &mut BTreeMap<String, ConfigValue>, symbol: &str) -> Result<(), String> {
    let mut queue = vec![symbol.to_string()];
    let mut visited = HashSet::new();
    while let Some(current) = queue.pop() {
        if !visited.insert(current.clone()) { continue; }
        let Some(opt) = schema.get(&current) else { continue; };
        for selected in &opt.selects {
            if let Some(target) = schema.get(selected) {
                let value = match target.ty {
                    ConfigType::Bool => ConfigValue::Bool(true),
                    ConfigType::Tristate => ConfigValue::Tristate('y'),
                    _ => target.default.clone(),
                };
                if values.get(selected) != Some(&value) {
                    values.insert(selected.clone(), value);
                    queue.push(selected.clone());
                }
            }
        }
    }
    Ok(())
}

fn prompt_key(prompt: &str) -> io::Result<Key> {
    let (rows, _) = terminal_size();
    let row = rows.saturating_sub(1).max(1);
    print!("\x1b[{row};1H\x1b[2K{} ", prompt);
    io::stdout().flush()?;
    read_key()
}

fn prompt_raw(prompt: &str, current: &str) -> io::Result<String> {
    let mut input = current.to_string();
    loop {
        let (rows, _) = terminal_size();
        let row = rows.saturating_sub(1).max(1);
        print!("\x1b[{row};1H\x1b[2K{}: {}", prompt, input);
        io::stdout().flush()?;
        match read_key()? {
            Key::Enter => return Ok(input),
            Key::Esc => return Ok(current.to_string()),
            Key::Backspace => { input.pop(); }
            Key::Char(ch) if !ch.is_control() => input.push(ch),
            _ => {}
        }
    }
}

fn run_line_menu(schema: &Schema, values: &mut BTreeMap<String, ConfigValue>) -> Result<MenuOutcome, String> {
    println!("Mirage configuration fallback menu");
    println!("stdin/stdout are not a terminal, so using line mode.");
    loop {
        println!();
        let mut displayed = Vec::new();
        for (idx, opt) in schema.options.iter().enumerate() {
            if !option_visible(opt, values) { continue; }
            let value = values.get(&opt.symbol).unwrap_or(&opt.default);
            println!("{:>3}. {:<12} {} ({})", displayed.len() + 1, value.short_display(), opt.prompt, opt.category);
            displayed.push(idx);
        }
        println!("Enter number to toggle/edit, s to save, q to discard:");
        let mut line = String::new();
        io::stdin().read_line(&mut line).map_err(|err| err.to_string())?;
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("s") { return Ok(MenuOutcome::Save); }
        if trimmed.eq_ignore_ascii_case("q") { return Ok(MenuOutcome::Discard); }
        if let Ok(number) = trimmed.parse::<usize>() {
            if let Some(idx) = displayed.get(number.saturating_sub(1)).copied() {
                let mut state = MenuState::new(schema);
                toggle_option(schema, values, idx, &mut state)?;
            }
        }
    }
}

fn generate_artifacts(schema: &Schema, values: &BTreeMap<String, ConfigValue>, out_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(out_dir).map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;
    let features = cargo_features(schema, values);
    let kernel_cmdline = kernel_cmdline(values);
    let display_args = if value_enabled(values.get("CONFIG_MIRAGE_HW_FRAMEBUFFER")) { "-display gtk" } else { "-display none" };
    let serial_args = if value_enabled(values.get("CONFIG_MIRAGE_SERIAL_CONSOLE")) { "-serial stdio" } else { "" };
    let debug_args = if value_enabled(values.get("CONFIG_MIRAGE_QEMU_DEBUG")) { "-S -s -d int,cpu_reset,guest_errors" } else { "" };

    let mut generated = String::new();
    generated.push_str("// Generated by mirageconfig. Do not edit.\n");
    for opt in &schema.options {
        let value = values.get(&opt.symbol).unwrap_or(&opt.default);
        generated.push_str("pub const ");
        generated.push_str(&opt.symbol);
        generated.push_str(": ");
        generated.push_str(rust_type(&opt.ty));
        generated.push_str(" = ");
        generated.push_str(&value.to_rust_literal());
        generated.push_str(";\n");
    }
    generated.push_str("pub const MIRAGE_CARGO_FEATURES: &[&str] = &[");
    for (idx, feature) in features.iter().enumerate() {
        if idx != 0 { generated.push_str(", "); }
        generated.push('"');
        generated.push_str(feature);
        generated.push('"');
    }
    generated.push_str("];\n");
    generated.push_str("pub const MIRAGE_KERNEL_CMDLINE: &str = ");
    generated.push_str(&ConfigValue::String(kernel_cmdline.clone()).to_rust_literal());
    generated.push_str(";\n");
    fs::write(out_dir.join("generated.rs"), generated).map_err(|err| err.to_string())?;

    fs::write(
        out_dir.join("cargo_features.env"),
        format!("MIRAGE_FEATURES={}\n", shell_quote(&features.join(" "))),
    ).map_err(|err| err.to_string())?;
    fs::write(
        out_dir.join("build_flags.env"),
        format!(
            "MIRAGE_QEMU_DISPLAY_ARGS={}\nMIRAGE_QEMU_SERIAL_ARGS={}\nMIRAGE_QEMU_DEBUG_ARGS={}\nMIRAGE_KERNEL_CMDLINE={}\n",
            shell_quote(display_args),
            shell_quote(serial_args),
            shell_quote(debug_args),
            shell_quote(&kernel_cmdline),
        ),
    ).map_err(|err| err.to_string())?;
    Ok(())
}

fn cargo_features(schema: &Schema, values: &BTreeMap<String, ConfigValue>) -> Vec<String> {
    let mut features = Vec::new();
    let mut seen = BTreeSet::new();
    for opt in &schema.options {
        if let Some(feature) = &opt.cargo_feature {
            if value_enabled(values.get(&opt.symbol)) && seen.insert(feature.clone()) {
                features.push(feature.clone());
            }
        }
    }
    features
}

fn kernel_cmdline(values: &BTreeMap<String, ConfigValue>) -> String {
    let mut parts = Vec::new();
    if value_enabled(values.get("CONFIG_MIRAGE_FULL_BOOT")) { parts.push("mirage.full_boot=1"); }
    if value_enabled(values.get("CONFIG_MIRAGE_VERBOSE_BOOT")) { parts.push("mirage.verbose=1"); }
    if value_enabled(values.get("CONFIG_MIRAGE_DEBUG")) { parts.push("mirage.debug=1"); }
    if value_enabled(values.get("CONFIG_MIRAGE_TRACE_IPC")) { parts.push("mirage.trace_ipc=1"); }
    if value_enabled(values.get("CONFIG_MIRAGE_HW_FRAMEBUFFER")) { parts.push("mirage.framebuffer=1"); }
    parts.join(" ")
}

fn rust_type(ty: &ConfigType) -> &'static str {
    match ty {
        ConfigType::Bool => "bool",
        ConfigType::Tristate => "char",
        ConfigType::String => "&str",
        ConfigType::Int => "i64",
        ConfigType::Hex => "u64",
    }
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() { return "''".to_string(); }
    let mut out = String::from("'");
    for ch in value.chars() {
        if ch == '\'' { out.push_str("'\\''"); } else { out.push(ch); }
    }
    out.push('\'');
    out
}

fn escape_config_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
