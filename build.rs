fn main() {
    // libcef.so e os recursos do CEF ficam ao lado do executável (ver scripts/bundle.sh).
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
}
