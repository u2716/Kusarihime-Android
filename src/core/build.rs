use std::env;
use std::path::{Path, PathBuf};

fn add_sources(build: &mut cc::Build, root: &Path, files: &[&str]) {
    for rel in files {
        let mut p = root.join(rel);
        p.set_extension("c");
        println!("cargo:rerun-if-changed={}", p.display());
        build.file(p);
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=FUJIHIME_SKIP_NATIVE_C_CHECK");
    println!("cargo:rerun-if-env-changed=IPHONEOS_DEPLOYMENT_TARGET");
    println!("cargo:rerun-if-env-changed=ANDROID_API");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_HOME");
    // Android: cc picks up NDK clang via cargo-ndk (CC_<target>). No -miphoneos flag here;
    // FreeType + pl_mpeg are plain C and compile unchanged for aarch64-linux-android.
    println!("cargo:rerun-if-changed=third_party/pl_mpeg/pl_mpeg.h");
    println!("cargo:rerun-if-changed=third_party/pl_mpeg/pl_mpeg_shim.c");
    println!("cargo:rerun-if-changed=third_party/freetype2_shim/ft_shim.c");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_abi = env::var("CARGO_CFG_TARGET_ABI").unwrap_or_default();
    let skip_native = env::var("FUJIHIME_SKIP_NATIVE_C_CHECK").ok().as_deref() == Some("1");
    let host_is_macos = cfg!(target_os = "macos");

    if target_os == "ios" && !host_is_macos {
        if skip_native {
            println!(
                "cargo:warning=FUJIHIME_SKIP_NATIVE_C_CHECK=1: skipping C compilation (FreeType 2 + pl_mpeg) for iOS target on non-macOS host"
            );
            return;
        }
        panic!(
            "Building for target OS 'ios' requires a macOS host with Xcode SDK to compile \
             FreeType 2 and pl_mpeg static libraries. Set FUJIHIME_SKIP_NATIVE_C_CHECK=1 only \
             when running Rust-only `cargo check --target aarch64-apple-ios` on Linux."
        );
    }

    let ios_min_flag = if target_os == "ios" {
        let ver = env::var("IPHONEOS_DEPLOYMENT_TARGET")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "14.4".to_string());
        if target_abi == "sim" {
            Some(format!("-mios-simulator-version-min={ver}"))
        } else {
            Some(format!("-miphoneos-version-min={ver}"))
        }
    } else {
        None
    };

    // 1. Compile MIT pl_mpeg C shim into static library `libpl_mpeg_shim.a`.
    let mut plm_build = cc::Build::new();
    plm_build
        .warnings(false)
        .include("third_party/pl_mpeg")
        .file("third_party/pl_mpeg/pl_mpeg_shim.c");
    if let Some(flag) = &ios_min_flag {
        plm_build.flag(flag);
    }
    plm_build.compile("pl_mpeg_shim");

    // 2. Compile bundled FreeType 2.13.2 + C shim into static library `libfreetype2_shim.a`.
    let ft_root = PathBuf::from("third_party/freetype2");
    let ft_src = ft_root.join("src");
    let mut ft_build = cc::Build::new();
    ft_build
        .warnings(false)
        .include(ft_root.join("include"))
        .include(&ft_src)
        .define("FT2_BUILD_LIBRARY", None)
        .file("third_party/freetype2_shim/ft_shim.c");
    if let Some(flag) = &ios_min_flag {
        ft_build.flag(flag);
    }

    add_sources(
        &mut ft_build,
        &ft_src,
        &[
            "autofit/autofit",
            "base/ftbase",
            "base/ftbbox",
            "base/ftbdf",
            "base/ftbitmap",
            "base/ftcid",
            "base/ftdebug",
            "base/ftfstype",
            "base/ftgasp",
            "base/ftglyph",
            "base/ftgxval",
            "base/ftinit",
            "base/ftmm",
            "base/ftotval",
            "base/ftpatent",
            "base/ftpfr",
            "base/ftstroke",
            "base/ftsynth",
            "base/ftsystem",
            "base/fttype1",
            "base/ftwinfnt",
            "bdf/bdf",
            "bzip2/ftbzip2",
            "cache/ftcache",
            "cff/cff",
            "cid/type1cid",
            "gzip/ftgzip",
            "lzw/ftlzw",
            "pcf/pcf",
            "pfr/pfr",
            "psaux/psaux",
            "pshinter/pshinter",
            "psnames/psnames",
            "raster/raster",
            "sdf/sdf",
            "svg/svg",
            "sfnt/sfnt",
            "smooth/smooth",
            "truetype/truetype",
            "type1/type1",
            "type42/type42",
            "winfonts/winfnt",
        ],
    );

    ft_build.compile("freetype2_shim");
}
