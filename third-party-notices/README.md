# Third-party notices

These notices cover linked Rust crates, the Rust standard library, and the MinGW runtime used by the current development build. They do not grant a license to FreePort's own source code. The project owner has not selected its license.

The compiled product has no third-party runtime DLL dependency. Statically linked dependencies still carry notice obligations. Regenerate and review this directory if the compiler, linker or Cargo.lock changes; archive the exact toolchain versions with a release.

`dependencies.json` inventories locked application crates; `COPYRIGHT-library.html` and `rust-licenses` contain the installed standard library's notices. The `gcc-libs`, `crt` and `libwinpthread` directories preserve the installed toolchain's runtime notices. Their presence is a conservative notice bundle, not a statement that every component is linked.
