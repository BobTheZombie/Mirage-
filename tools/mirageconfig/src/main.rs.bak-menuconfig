use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

const DEFAULT_SCHEMA: &str = "config/MirageConfig.toml";
const DEFAULT_CONFIG: &str = "mirage.conf";
const DEFAULT_OUT_DIR: &str = "target/mirage/config";

#[derive(Clone, Debug, PartialEq, Eq)]
enum ConfigType {
    Bool,
    Tristate,
    String,
    Int,
    Hex,
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
    fn as_bool(&self) -> Option<bool> {
        match self {
            ConfigValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    fn to_config_text(&self) -> String {
        match self {
            ConfigValue::Bool(true) => "y".to_string(),
            ConfigValue::Bool(false) => "n".to_string(),
            ConfigValue::Tristate(value) => value.to_string(),
            ConfigValue::String(value) => format!("\"{}\"", value.replace('"', "\\\"")),
            ConfigValue::Int(value) => value.to_string(),
            ConfigValue::Hex(value) => format!("0x{value:x}"),
        }
    }

    fn to_rust_literal(&self) -> String {
        match self {
            ConfigValue::Bool(value) => value.to_string(),
            ConfigValue::Tristate(value) => format!("'{}'", value),
            ConfigValue::String(value) => format!("\"{}\"", value.replace('"', "\\\"")),
            ConfigValue::Int(value) => format!("{}i64", value),
            ConfigValue::Hex(value) => format!("0x{value:x}u64"),
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

    fn defaults(&self) -> BTreeMap<String, ConfigValue> {
        self.options
            .iter()
            .map(|opt| (opt.symbol.clone(), opt.default.clone()))
            .collect()
    }

    fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let required_categories = [
            "Architecture",
            "Boot",
            "Hardware",
            "Kernel Core",
            "Memory",
            "Debug",
        ];
        for category in required_categories {
            if !self.options.iter().any(|opt| opt.category == category) {
                errors.push(format!("schema is missing required category '{category}'"));
            }
        }
        for opt in &self.options {
            if !opt.symbol.starts_with("CONFIG_") {
                errors.push(format!("{}: symbol must start with CONFIG_", opt.symbol));
            }
            if opt.prompt.is_empty() {
                errors.push(format!("{}: prompt is required", opt.symbol));
            }
            if opt.help.is_empty() {
                errors.push(format!("{}: help is required", opt.symbol));
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
            if !matches!(
                (&opt.ty, &opt.default),
                (ConfigType::Bool, ConfigValue::Bool(_))
            ) {
                errors.push(format!("{}: default value does not match type", opt.symbol));
            }
        }
        errors.extend(self.select_cycles());
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
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
                errors.push(format!(
                    "circular select dependency: {}",
                    cycle.join(" -> ")
                ));
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
            visit(
                self,
                &opt.symbol,
                &mut Vec::new(),
                &mut visited,
                &mut errors,
            );
        }
        errors.sort();
        errors.dedup();
        errors
    }
}

#[derive(Clone, Debug)]
enum Line {
    Assignment { symbol: String },
    Other(String),
}

#[derive(Clone, Debug)]
struct ParsedConfig {
    values: BTreeMap<String, ConfigValue>,
    lines: Vec<Line>,
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

    let command_count = [
        args.menu,
        args.defconfig,
        args.oldconfig,
        args.savedefconfig,
        args.list,
        args.check,
    ]
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
        write_config(output, &schema, &values, None, false)?;
        if args.generate {
            generate_artifacts(&schema, &values, &args.out_dir)?;
        }
        return Ok(());
    }

    if args.check {
        let parsed = read_config(&args.config, &schema)?;
        validate_values(&schema, &parsed.values, true)?;
        let values = resolve_selects(&schema, parsed.values.clone())?;
        validate_values(&schema, &values, true)?;
        if args.generate {
            generate_artifacts(&schema, &values, &args.out_dir)?;
        }
        return Ok(());
    }

    if args.oldconfig {
        let parsed = read_config_if_present(&args.config, &schema)?;
        let mut values = schema.defaults();
        let lines = parsed.as_ref().map(|parsed| parsed.lines.clone());
        if let Some(parsed) = parsed {
            for (symbol, value) in parsed.values {
                values.insert(symbol, value);
            }
        }
        let values = resolve_selects(&schema, values)?;
        validate_values(&schema, &values, false)?;
        let output = args.output.as_deref().unwrap_or(&args.config);
        write_config(output, &schema, &values, lines.as_deref(), false)?;
        if args.generate {
            generate_artifacts(&schema, &values, &args.out_dir)?;
        }
        return Ok(());
    }

    if args.savedefconfig {
        let parsed = read_config(&args.config, &schema)?;
        let values = resolve_selects(&schema, parsed.values)?;
        validate_values(&schema, &values, true)?;
        let output = args
            .output
            .unwrap_or_else(|| PathBuf::from("mirage.defconfig"));
        write_config(&output, &schema, &values, None, true)?;
        if args.generate {
            generate_artifacts(&schema, &values, &args.out_dir)?;
        }
        return Ok(());
    }

    if args.menu {
        let parsed = read_config_if_present(&args.config, &schema)?;
        let mut values = schema.defaults();
        let lines = parsed.as_ref().map(|parsed| parsed.lines.clone());
        if let Some(parsed) = parsed {
            for (symbol, value) in parsed.values {
                values.insert(symbol, value);
            }
        }
        menu(&schema, &mut values)?;
        let values = resolve_selects(&schema, values)?;
        validate_values(&schema, &values, false)?;
        let output = args.output.as_deref().unwrap_or(&args.config);
        write_config(output, &schema, &values, lines.as_deref(), false)?;
        if args.generate {
            generate_artifacts(&schema, &values, &args.out_dir)?;
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
            "--menu" => args.menu = true,
            "--defconfig" => args.defconfig = true,
            "--oldconfig" => args.oldconfig = true,
            "--savedefconfig" => args.savedefconfig = true,
            "--list" => args.list = true,
            "--check" => args.check = true,
            "--generate" => args.generate = true,
            "--config" => args.config = PathBuf::from(next_arg(&mut iter, "--config")?),
            "--output" => args.output = Some(PathBuf::from(next_arg(&mut iter, "--output")?)),
            "--schema" => args.schema = PathBuf::from(next_arg(&mut iter, "--schema")?),
            "--out-dir" => args.out_dir = PathBuf::from(next_arg(&mut iter, "--out-dir")?),
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(args)
}

fn next_arg(iter: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!("Mirage configuration utility");
    println!("  --menu              interactive bool menu");
    println!("  --defconfig         write defaults");
    println!("  --oldconfig         merge existing config with schema defaults");
    println!("  --savedefconfig     write only values differing from defaults");
    println!("  --list              list schema options");
    println!("  --check             validate config");
    println!("  --config <file>     input/output config file (default mirage.conf)");
    println!("  --output <file>     output file for writing commands");
    println!("  --generate          generate target/mirage/config artifacts");
}

fn parse_schema_toml(text: &str) -> Result<Schema, String> {
    let mut tables: Vec<BTreeMap<String, TomlValue>> = Vec::new();
    let mut current: Option<BTreeMap<String, TomlValue>> = None;

    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let stripped = strip_toml_comment(raw_line);
        let line = stripped.trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[options]]" {
            if let Some(table) = current.take() {
                tables.push(table);
            }
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
    if let Some(table) = current.take() {
        tables.push(table);
    }

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
        let default = parse_default(
            &ty,
            table
                .get("default")
                .ok_or_else(|| format!("{symbol}: missing default"))?,
        )?;
        let option = OptionDef {
            prompt: required_string(&table, "prompt", idx)?,
            category: required_string(&table, "category", idx)?,
            help: required_string(&table, "help", idx)?,
            depends_on: required_array(&table, "depends_on", idx)?,
            selects: required_array(&table, "selects", idx)?,
            visible_if: required_array(&table, "visible_if", idx)?,
            cargo_feature: optional_string(&table, "cargo_feature")?,
            symbol: symbol.clone(),
            default,
            ty,
        };
        by_symbol.insert(symbol, options.len());
        options.push(option);
    }

    if options.is_empty() {
        return Err("schema contains no [[options]] entries".to_string());
    }

    Ok(Schema { options, by_symbol })
}

#[derive(Clone, Debug)]
enum TomlValue {
    String(String),
    Bool(bool),
    Array(Vec<String>),
}

fn strip_toml_comment(line: &str) -> String {
    let mut in_string = false;
    let mut escaped = false;
    let mut output = String::new();
    for ch in line.chars() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            output.push(ch);
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            output.push(ch);
            continue;
        }
        if ch == '#' && !in_string {
            break;
        }
        output.push(ch);
    }
    output
}

fn parse_toml_value(text: &str) -> Result<TomlValue, String> {
    if let Some(value) = parse_quoted(text)? {
        return Ok(TomlValue::String(value));
    }
    match text {
        "true" => return Ok(TomlValue::Bool(true)),
        "false" => return Ok(TomlValue::Bool(false)),
        _ => {}
    }
    if text.starts_with('[') && text.ends_with(']') {
        let inner = &text[1..text.len() - 1];
        let mut values = Vec::new();
        if inner.trim().is_empty() {
            return Ok(TomlValue::Array(values));
        }
        for item in split_array(inner)? {
            let value = parse_quoted(item.trim())?
                .ok_or_else(|| format!("array item must be a quoted string: {item}"))?;
            values.push(value);
        }
        return Ok(TomlValue::Array(values));
    }
    Err(format!("unsupported TOML value '{text}'"))
}

fn parse_quoted(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if !text.starts_with('"') {
        return Ok(None);
    }
    if !text.ends_with('"') || text.len() < 2 {
        return Err(format!("unterminated string {text}"));
    }
    let inner = &text[1..text.len() - 1];
    let mut value = String::new();
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            value.push(match ch {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            value.push(ch);
        }
    }
    if escaped {
        return Err("unterminated escape".to_string());
    }
    Ok(Some(value))
}

fn split_array(inner: &str) -> Result<Vec<&str>, String> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if ch == ',' && !in_string {
            result.push(&inner[start..idx]);
            start = idx + 1;
        }
    }
    if in_string {
        return Err("unterminated string in array".to_string());
    }
    result.push(&inner[start..]);
    Ok(result)
}

fn required_string(
    table: &BTreeMap<String, TomlValue>,
    key: &str,
    idx: usize,
) -> Result<String, String> {
    match table.get(key) {
        Some(TomlValue::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("option #{idx}: {key} must be a string")),
        None => Err(format!("option #{idx}: missing {key}")),
    }
}

fn optional_string(
    table: &BTreeMap<String, TomlValue>,
    key: &str,
) -> Result<Option<String>, String> {
    match table.get(key) {
        Some(TomlValue::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{key} must be a string")),
        None => Ok(None),
    }
}

fn required_array(
    table: &BTreeMap<String, TomlValue>,
    key: &str,
    idx: usize,
) -> Result<Vec<String>, String> {
    match table.get(key) {
        Some(TomlValue::Array(value)) => Ok(value.clone()),
        Some(_) => Err(format!("option #{idx}: {key} must be an array")),
        None => Err(format!("option #{idx}: missing {key}")),
    }
}

fn parse_default(ty: &ConfigType, value: &TomlValue) -> Result<ConfigValue, String> {
    match (ty, value) {
        (ConfigType::Bool, TomlValue::Bool(value)) => Ok(ConfigValue::Bool(*value)),
        (ConfigType::Tristate, TomlValue::String(value))
            if matches!(value.as_str(), "y" | "m" | "n") =>
        {
            Ok(ConfigValue::Tristate(value.chars().next().unwrap()))
        }
        (ConfigType::String, TomlValue::String(value)) => Ok(ConfigValue::String(value.clone())),
        (ConfigType::Int, TomlValue::String(value)) => value
            .parse::<i64>()
            .map(ConfigValue::Int)
            .map_err(|err| format!("invalid int default: {err}")),
        (ConfigType::Hex, TomlValue::String(value)) => parse_hex(value).map(ConfigValue::Hex),
        _ => Err("default does not match option type".to_string()),
    }
}

fn read_config_if_present(path: &Path, schema: &Schema) -> Result<Option<ParsedConfig>, String> {
    if path.exists() {
        read_config(path, schema).map(Some)
    } else {
        Ok(None)
    }
}

fn read_config(path: &Path, schema: &Schema) -> Result<ParsedConfig, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read config {}: {err}", path.display()))?;
    parse_config_text(&text, schema)
}

fn parse_config_text(text: &str, schema: &Schema) -> Result<ParsedConfig, String> {
    let mut values = BTreeMap::new();
    let mut lines = Vec::new();
    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            if let Some(symbol) = parse_not_set_comment(trimmed) {
                let opt = schema
                    .get(&symbol)
                    .ok_or_else(|| format!("line {line_no}: unknown symbol {symbol}"))?;
                values.insert(symbol.clone(), parse_config_value(opt, "n")?);
                lines.push(Line::Assignment { symbol });
            } else {
                lines.push(Line::Other(raw_line.to_string()));
            }
            continue;
        }
        let (symbol, value_text) = trimmed
            .split_once('=')
            .ok_or_else(|| format!("line {line_no}: expected CONFIG_SYMBOL=value"))?;
        let symbol = symbol.trim().to_string();
        if !symbol.starts_with("CONFIG_") {
            return Err(format!("line {line_no}: expected CONFIG_ symbol"));
        }
        let opt = schema
            .get(&symbol)
            .ok_or_else(|| format!("line {line_no}: unknown symbol {symbol}"))?;
        let value_text = value_text.split('#').next().unwrap_or(value_text).trim();
        let value = parse_config_value(opt, value_text)
            .map_err(|err| format!("line {line_no}: {symbol}: {err}"))?;
        values.insert(symbol.clone(), value);
        lines.push(Line::Assignment { symbol });
    }
    Ok(ParsedConfig { values, lines })
}

fn parse_not_set_comment(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("# ")?;
    let symbol = rest.strip_suffix(" is not set")?;
    if symbol.starts_with("CONFIG_") {
        Some(symbol.to_string())
    } else {
        None
    }
}

fn parse_config_value(opt: &OptionDef, text: &str) -> Result<ConfigValue, String> {
    match opt.ty {
        ConfigType::Bool => match text {
            "y" | "Y" | "1" | "true" => Ok(ConfigValue::Bool(true)),
            "n" | "N" | "0" | "false" => Ok(ConfigValue::Bool(false)),
            other => Err(format!("invalid bool value '{other}' (expected y or n)")),
        },
        ConfigType::Tristate => match text {
            "y" | "m" | "n" => Ok(ConfigValue::Tristate(text.chars().next().unwrap())),
            other => Err(format!("invalid tristate value '{other}'")),
        },
        ConfigType::String => parse_quoted(text)
            .and_then(|value| value.ok_or_else(|| "string values must be quoted".to_string()))
            .map(ConfigValue::String),
        ConfigType::Int => text
            .parse::<i64>()
            .map(ConfigValue::Int)
            .map_err(|err| format!("invalid int value: {err}")),
        ConfigType::Hex => parse_hex(text).map(ConfigValue::Hex),
    }
}

fn parse_hex(text: &str) -> Result<u64, String> {
    let trimmed = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    u64::from_str_radix(trimmed, 16).map_err(|err| format!("invalid hex value: {err}"))
}

fn resolve_selects(
    schema: &Schema,
    mut values: BTreeMap<String, ConfigValue>,
) -> Result<BTreeMap<String, ConfigValue>, String> {
    for opt in &schema.options {
        values
            .entry(opt.symbol.clone())
            .or_insert_with(|| opt.default.clone());
    }
    let mut changed = true;
    while changed {
        changed = false;
        for opt in &schema.options {
            if values.get(&opt.symbol).and_then(ConfigValue::as_bool) == Some(true) {
                for selected in &opt.selects {
                    if values.get(selected).and_then(ConfigValue::as_bool) != Some(true) {
                        values.insert(selected.clone(), ConfigValue::Bool(true));
                        changed = true;
                    }
                }
            }
        }
    }
    Ok(values)
}

fn validate_values(
    schema: &Schema,
    values: &BTreeMap<String, ConfigValue>,
    require_all: bool,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for symbol in values.keys() {
        if schema.get(symbol).is_none() {
            errors.push(format!("unknown symbol {symbol}"));
        }
    }
    for opt in &schema.options {
        let Some(value) = values.get(&opt.symbol) else {
            if require_all {
                errors.push(format!("missing required option {}", opt.symbol));
            }
            continue;
        };
        match (&opt.ty, value) {
            (ConfigType::Bool, ConfigValue::Bool(_)) => {}
            (ConfigType::Tristate, ConfigValue::Tristate(_)) => {}
            (ConfigType::String, ConfigValue::String(_)) => {}
            (ConfigType::Int, ConfigValue::Int(_)) => {}
            (ConfigType::Hex, ConfigValue::Hex(_)) => {}
            _ => errors.push(format!(
                "{} has a value that does not match its type",
                opt.symbol
            )),
        }
        if value.as_bool() == Some(true) {
            for dep in &opt.depends_on {
                if values.get(dep).and_then(ConfigValue::as_bool) != Some(true) {
                    errors.push(format!("{} depends on {dep}", opt.symbol));
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn write_config(
    path: &Path,
    schema: &Schema,
    values: &BTreeMap<String, ConfigValue>,
    existing_lines: Option<&[Line]>,
    minimal: bool,
) -> Result<(), String> {
    let mut output = String::new();
    let mut written = BTreeSet::new();
    if !minimal {
        output.push_str("# Mirage kernel configuration\n");
        output.push_str("# Generated by tools/mirageconfig from config/MirageConfig.toml\n\n");
    }

    if let Some(lines) = existing_lines {
        for line in lines {
            match line {
                Line::Other(text) => {
                    if !text.starts_with("# Mirage kernel configuration")
                        && !text.starts_with("# Generated by tools/mirageconfig")
                    {
                        output.push_str(text);
                        output.push('\n');
                    }
                }
                Line::Assignment { symbol } => {
                    if written.contains(symbol) {
                        continue;
                    }
                    if let Some(opt) = schema.get(symbol) {
                        if let Some(value) = values.get(symbol) {
                            if minimal && value == &opt.default {
                                continue;
                            }
                            output.push_str(&format_assignment(symbol, value));
                            written.insert(symbol.clone());
                        }
                    }
                }
            }
        }
    }

    let mut current_category = String::new();
    for opt in &schema.options {
        if written.contains(&opt.symbol) {
            continue;
        }
        let value = values.get(&opt.symbol).unwrap_or(&opt.default);
        if minimal && value == &opt.default {
            continue;
        }
        if !minimal && current_category != opt.category {
            current_category = opt.category.clone();
            output.push_str(&format!("\n# {}\n", current_category));
        }
        output.push_str(&format_assignment(&opt.symbol, value));
        written.insert(opt.symbol.clone());
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::write(path, output).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn format_assignment(symbol: &str, value: &ConfigValue) -> String {
    format!("{symbol}={}\n", value.to_config_text())
}

fn list_config(schema: &Schema) {
    let mut current_category = "";
    for opt in &schema.options {
        if current_category != opt.category {
            current_category = &opt.category;
            println!("\n[{current_category}]");
        }
        let default = opt.default.to_config_text();
        let feature = opt
            .cargo_feature
            .as_ref()
            .map(|feature| format!(" feature={feature}"))
            .unwrap_or_default();
        println!(
            "{} ({}) default={}{}",
            opt.symbol, opt.prompt, default, feature
        );
        if !opt.depends_on.is_empty() {
            println!("  depends_on: {}", opt.depends_on.join(", "));
        }
        if !opt.selects.is_empty() {
            println!("  selects: {}", opt.selects.join(", "));
        }
    }
}

fn menu(schema: &Schema, values: &mut BTreeMap<String, ConfigValue>) -> Result<(), String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        println!("Non-interactive terminal detected; using oldconfig-style defaults.");
        return Ok(());
    }
    loop {
        println!("\nMirage configuration menu (enter number to toggle, s to save, q to quit)");
        let mut visible = Vec::new();
        let mut current_category = "";
        for opt in &schema.options {
            if !is_visible(opt, values) {
                continue;
            }
            if current_category != opt.category {
                current_category = &opt.category;
                println!("\n[{current_category}]");
            }
            let idx = visible.len() + 1;
            let enabled = values
                .get(&opt.symbol)
                .and_then(ConfigValue::as_bool)
                .unwrap_or(false);
            println!(
                "  {idx:2}. [{}] {}",
                if enabled { 'y' } else { 'n' },
                opt.prompt
            );
            println!("      {}", opt.symbol);
            visible.push(opt.symbol.clone());
        }
        print!("choice> ");
        io::stdout().flush().map_err(|err| err.to_string())?;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|err| err.to_string())?;
        let input = input.trim();
        if matches!(input, "s" | "q" | "") {
            break;
        }
        let choice = input
            .parse::<usize>()
            .map_err(|_| "expected a number, s, or q".to_string())?;
        let symbol = visible
            .get(choice.saturating_sub(1))
            .ok_or_else(|| "choice out of range".to_string())?;
        let old = values
            .get(symbol)
            .and_then(ConfigValue::as_bool)
            .unwrap_or(false);
        values.insert(symbol.clone(), ConfigValue::Bool(!old));
        *values = resolve_selects(schema, values.clone())?;
    }
    Ok(())
}

fn is_visible(opt: &OptionDef, values: &BTreeMap<String, ConfigValue>) -> bool {
    opt.visible_if
        .iter()
        .all(|symbol| values.get(symbol).and_then(ConfigValue::as_bool) == Some(true))
}

fn generate_artifacts(
    schema: &Schema,
    values: &BTreeMap<String, ConfigValue>,
    out_dir: &Path,
) -> Result<(), String> {
    fs::create_dir_all(out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;
    let mut generated_rs = String::new();
    generated_rs.push_str("// Generated by tools/mirageconfig. Do not edit.\n");
    generated_rs.push_str("\n");
    for opt in &schema.options {
        let value = values.get(&opt.symbol).unwrap_or(&opt.default);
        let rust_ty = match opt.ty {
            ConfigType::Bool => "bool",
            ConfigType::Tristate => "char",
            ConfigType::String => "&str",
            ConfigType::Int => "i64",
            ConfigType::Hex => "u64",
        };
        generated_rs.push_str(&format!(
            "pub const {}: {} = {};\n",
            opt.symbol,
            rust_ty,
            value.to_rust_literal()
        ));
    }
    fs::write(out_dir.join("generated.rs"), generated_rs)
        .map_err(|err| format!("failed to write generated.rs: {err}"))?;

    let features = cargo_features(schema, values);
    fs::write(
        out_dir.join("cargo_features.env"),
        format!(
            "MIRAGE_FEATURES=\"{}\"\n",
            shell_escape_env_value(&features.join(" "))
        ),
    )
    .map_err(|err| format!("failed to write cargo_features.env: {err}"))?;

    let serial = enabled(values, "CONFIG_MIRAGE_SERIAL_CONSOLE");
    let framebuffer = enabled(values, "CONFIG_MIRAGE_HW_FRAMEBUFFER");
    let qemu_debug = enabled(values, "CONFIG_MIRAGE_QEMU_DEBUG");
    let verbose = enabled(values, "CONFIG_MIRAGE_VERBOSE_BOOT");
    let qemu_serial_args = if serial { "-serial stdio" } else { "" };
    let qemu_display_args = if framebuffer { "" } else { "-display none" };
    let qemu_debug_args = if qemu_debug {
        "-S -s -d int,cpu_reset -D build/qemu.log"
    } else {
        ""
    };
    let kernel_cmdline = if verbose { "mirage.verbose=1" } else { "" };
    let build_flags = format!(
        concat!(
            "MIRAGE_QEMU_GRAPHICAL={}\n",
            "MIRAGE_QEMU_SERIAL_ARGS=\"{}\"\n",
            "MIRAGE_QEMU_DISPLAY_ARGS=\"{}\"\n",
            "MIRAGE_QEMU_DEBUG_ARGS=\"{}\"\n",
            "MIRAGE_KERNEL_CMDLINE=\"{}\"\n",
        ),
        if framebuffer { 1 } else { 0 },
        qemu_serial_args,
        qemu_display_args,
        qemu_debug_args,
        kernel_cmdline,
    );
    fs::write(out_dir.join("build_flags.env"), build_flags)
        .map_err(|err| format!("failed to write build_flags.env: {err}"))?;

    Ok(())
}

fn cargo_features(schema: &Schema, values: &BTreeMap<String, ConfigValue>) -> Vec<String> {
    let mut features = Vec::new();
    for opt in &schema.options {
        if opt.cargo_feature.is_some()
            && values.get(&opt.symbol).and_then(ConfigValue::as_bool) == Some(true)
        {
            features.push(opt.cargo_feature.clone().unwrap());
        }
    }
    features
}

fn enabled(values: &BTreeMap<String, ConfigValue>, symbol: &str) -> bool {
    values.get(symbol).and_then(ConfigValue::as_bool) == Some(true)
}

fn shell_escape_env_value(value: &str) -> String {
    value.replace('"', "\\\"")
}
