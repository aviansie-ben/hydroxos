#[cfg_attr(feature = "real_arch_api", allow(unused))]
mod api;

cfg_if::cfg_if! {
    if #[cfg(not(feature = "real_arch_api"))] {
        pub use api::*;
    } else if #[cfg(target_arch = "x86_64")] {
        pub mod x86_64;
        pub use self::x86_64::*;
    }
}
