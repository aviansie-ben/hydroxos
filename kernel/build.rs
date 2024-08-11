fn main() {
    println!("cargo::rustc-link-arg=-Tlinker.ld");
    println!("cargo::rerun-if-changed=linker.ld");
    println!("cargo::rerun-if-env-changed=HYDROXOS_OPTIONS");
}
