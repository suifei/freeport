fn main() {
    if savemycdrive::elevation::maybe_worker() {
        return;
    }
    if savemycdrive::handles::maybe_helper() {
        return;
    }
    if let Err(e) = savemycdrive::cli::run() {
        eprintln!(
            "SaveMyCDrive: {}",
            savemycdrive::i18n::describe_error(e.as_ref())
        );
        std::process::exit(1);
    }
}
