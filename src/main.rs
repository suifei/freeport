fn main() {
    if freeport::elevation::maybe_worker() {
        return;
    }
    if freeport::handles::maybe_helper() {
        return;
    }
    if let Err(e) = freeport::cli::run() {
        eprintln!("FreePort: {}", freeport::i18n::describe_error(e.as_ref()));
        std::process::exit(1);
    }
}
