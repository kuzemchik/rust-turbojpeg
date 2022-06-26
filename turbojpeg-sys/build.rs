#![allow(unused_imports, dead_code)]
use anyhow::{Result, Context as _, anyhow, bail};
use std::{env, fs};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    let library = build_or_find_library()?;
    generate_or_copy_bindings(&library)?;
    Ok(())
}

#[derive(Debug)]
struct Library {
    include_paths: Vec<PathBuf>,
    defines: HashMap<String, Option<String>>,
}

fn build_or_find_library() -> Result<Library> {
    match env("TURBOJPEG_SOURCE") {
        Some(source) => {
            if source.eq_ignore_ascii_case("vendor") {
                build_vendor()
            } else if source.eq_ignore_ascii_case("pkg-config") ||
                source.eq_ignore_ascii_case("pkgconfig") ||
                source.eq_ignore_ascii_case("pkgconf")
            {
                find_pkg_config()
            } else if source.eq_ignore_ascii_case("explicit") {
                find_explicit()
            } else {
                bail!("Unknown value of TURBOJPEG_SOURCE, supported values are:\n\
                    - 'vendor' to build the library from source bundled with the turbojpeg-sys crate,\n\
                    - 'pkg-config' to find the library using pkg-config,\n\
                    - 'explicit' to use TURBOJPEG_LIB_DIR and TURBOJPEG_INCLUDE_DIR")
            }
        },
        None => {
            if cfg!(feature = "cmake") {
                build_vendor()
            } else if cfg!(feature = "pkg-config") {
                find_pkg_config()
            } else {
                find_explicit()
            }
        },
    }
}

#[cfg(feature = "pkg-config")]
fn find_pkg_config() -> Result<Library> {
    println!("Using pkg-config to find libturbojpeg");
    let lib = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("libturbojpeg")
        .context("could not find turbojpeg using pkg-config")?;

    Ok(Library {
        include_paths: lib.include_paths,
        defines: lib.defines,
    })
}

#[cfg(not(feature = "pkg-config"))]
fn find_pkg_config() -> Result<Library> {
    bail!("Trying to find turbojpeg using pkg-config, but the `pkg-config` feature is disabled. \
        You have two options:\n\
        - enable `pkg-config` feature of `turbojpeg-sys` crate\n\
        - use TURBOJPEG_SOURCE to select other source for the library")
}

fn find_explicit() -> Result<Library> {
    println!("Using TURBOJPEG_LIB_DIR and TURBOJPEG_INCLUDE_DIR to find turbojpeg");

    let lib_dir = env_path("TURBOJPEG_LIB_DIR")
        .or_else(|| env_path("TURBOJPEG_LIB_PATH"))
        .context("TURBOJPEG_SOURCE is set to 'explicit', but TURBOJPEG_LIB_DIR is not set")?;
    let include_dir = env_path("TURBOJPEG_INCLUDE_DIR")
        .or_else(|| env_path("TURBOJPEG_INCLUDE_PATH"))
        .context("TURBOJPEG_SOURCE is set to 'explicit', but TURBOJPEG_INCLUDE_DIR is not set")?;

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=turbojpeg");
    Ok(Library {
        include_paths: vec![include_dir],
        defines: HashMap::new(),
    })
}


#[cfg(feature = "cmake")]
fn build_vendor() -> Result<Library> {
    println!("Building turbojpeg from source");

    let source_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("libjpeg-turbo");
    let mut cmake = cmake::Config::new(source_path);
    let dst_path = cmake.build();

    let lib_path = dst_path.join("lib");
    let include_path = dst_path.join("include");
    println!("cargo:rustc-link-search=native={}", lib_path.display());
    println!("cargo:rustc-link-lib=static=turbojpeg");
    Ok(Library {
        include_paths: vec![include_path],
        defines: HashMap::new(),
    })
}

#[cfg(not(feature = "cmake"))]
fn build_vendor() -> Result<()> {
    bail!("Trying to build turbojpeg from source, but the `cmake` feature is disabled.\
        You have two options:\n\
        - enable `cmake` feature of `turbojpeg-sys` crate\n\
        - use TURBOJPEG_SOURCE to select other source for the library")
}



fn generate_or_copy_bindings(library: &Library) -> Result<()> {
    match env("TURBOJPEG_BINDING") {
        Some(binding) => {
            if binding.eq_ignore_ascii_case("pregenerated") {
                copy_pregenerated_bindings()
            } else if binding.eq_ignore_ascii_case("bindgen") {
                generate_bindings(library)
            } else {
                bail!("Unknown value of TURBOJPEG_BINDING, supported values are:\n\
                    - `pregenerated` to use our pregenerated Rust bindings,\n\
                    - `bindgen` to generate the bindings with bindgen")
            }
        },
        None => {
            if cfg!(feature = "bindgen") {
                generate_bindings(library)
            } else {
                copy_pregenerated_bindings()
            }
        },
    }
}

fn copy_pregenerated_bindings() -> Result<()> {
    println!("Using pregenerated bindings");
    let out_path = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let crate_path = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    fs::copy(crate_path.join("bindings.rs"), out_path.join("bindings.rs"))?;
    println!("cargo:rerun-if-changed={}", crate_path.join("bindings.rs").to_str().unwrap());
    Ok(())
}

#[cfg(feature = "bindgen")]
fn generate_bindings(library: &Library) -> Result<()> {
    println!("Generating bindings using bindgen");

    let target = env::var("TARGET").unwrap();
    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .use_core()
        .ctypes_prefix("libc")
        .clang_args(&["-target", &target]);

    for path in library.include_paths.iter() {
        let path = path.to_str().unwrap();
        builder = builder.clang_arg(format!("-I{}", path));
        println!("cargo:rerun-if-changed={}", path);
    }

    for (name, value) in library.defines.iter() {
        if let Some(value) = value {
            builder = builder.clang_arg(format!("-D{}={}", name, value));
        } else {
            builder = builder.clang_arg(format!("-D{}", name));
        }
    }

    let bindings = builder.generate()
        .map_err(|_| anyhow!("could not generate bindings"))?;

    let out_file = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(&out_file)
        .context("could not write bindings to OUT_DIR")?;
    println!("Generated bindings are stored in {}", out_file.display());

    Ok(())
}

#[cfg(not(feature = "bindgen"))]
fn generate_bindings(_: &Library) -> Result<()> {
    bail!("Trying to build bindings with bindgen, but the `bindgen` feature is disabled. \
        You have two options:\n\
        - enable `bindgen` feature of `turbojpeg-sys` crate\n\
        - use TURBOJPEG_BINDING to select other method to obtain the bindings")
}



fn env(name: &str) -> Option<OsString> {
    // adapted from `openssl-sys` crate

    fn env_inner(name: &str) -> Option<OsString> {
        let value = env::var_os(name);
        println!("cargo:rerun-if-env-changed={}", name);

        match value {
            Some(ref v) => println!("{} = {}", name, v.to_string_lossy()),
            None => println!("{} unset", name),
        }

        value
    }

    let prefix = env::var("TARGET").unwrap().to_uppercase().replace('-', "_");
    let prefixed = format!("{}_{}", prefix, name);
    env_inner(&prefixed).or_else(|| env_inner(name))
}

fn env_bool(name: &str) -> Result<Option<bool>> {
    match env(name) {
        Some(value) => {
            for v in ["", "1", "yes", "true", "on"].into_iter() {
                if value.eq_ignore_ascii_case(v) { return Ok(Some(true)) }
            }
            for v in ["0", "no", "false", "off"].into_iter() {
                if value.eq_ignore_ascii_case(v) { return Ok(Some(false)) }
            }
            bail!("Env variable {} has value {:?}, expected empty or boolean", name, value)
        },
        None => Ok(None),
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env(name).map(|v| v.into())
}
