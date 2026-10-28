# Third-party notices

These notices cover linked Rust crates, the Rust standard library, and the MinGW runtime used by the build. FreePort's own code is licensed separately under MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).

The compiled product has no third-party runtime DLL dependency. Statically linked dependencies still carry notice obligations. Regenerate and review this directory if the compiler, linker or Cargo.lock changes; archive the exact toolchain versions with a release.

`dependencies.json` inventories locked application crates; release archives keep only the crates linked into the EXEs (`scripts/package-release.py`); `COPYRIGHT-library.html` and `rust-licenses` contain the installed standard library's notices. The `gcc-libs`, `crt` and `libwinpthread` directories preserve the installed toolchain's runtime notices. Their presence is a conservative notice bundle, not a statement that every component is linked.
