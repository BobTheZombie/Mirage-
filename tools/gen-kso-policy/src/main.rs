fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("kernel/startup/policies");
    let output = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("src/kernel/kso/generated.rs");
    match gen_kso_policy::generate_from_dir(input, output) {
        Ok(()) => {}
        Err(err) => {
            eprintln!("gen-kso-policy: {err}");
            std::process::exit(1);
        }
    }
}
