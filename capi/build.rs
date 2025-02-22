#![allow(clippy::let_unit_value)]

use std::env;
use std::ffi::OsStr;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Result;
use std::ops::Deref as _;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;


/// Format a command with the given list of arguments as a string.
fn format_command<C, A, S>(command: C, args: A) -> String
where
    C: AsRef<OsStr>,
    A: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter().fold(
        command.as_ref().to_string_lossy().into_owned(),
        |mut cmd, arg| {
            cmd += " ";
            cmd += arg.as_ref().to_string_lossy().deref();
            cmd
        },
    )
}

/// Run a command with the provided arguments.
fn run<C, A, S>(command: C, args: A) -> Result<()>
where
    C: AsRef<OsStr>,
    A: IntoIterator<Item = S> + Clone,
    S: AsRef<OsStr>,
{
    let instance = Command::new(command.as_ref())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .env_clear()
        .envs(env::vars().filter(|(k, _)| k == "PATH"))
        .args(args.clone())
        .output()
        .map_err(|err| {
            Error::new(
                ErrorKind::Other,
                format!(
                    "failed to run `{}`: {err}",
                    format_command(command.as_ref(), args.clone())
                ),
            )
        })?;

    if !instance.status.success() {
        let code = if let Some(code) = instance.status.code() {
            format!(" ({code})")
        } else {
            " (terminated by signal)".to_string()
        };

        let stderr = String::from_utf8_lossy(&instance.stderr);
        let stderr = stderr.trim_end();
        let stderr = if !stderr.is_empty() {
            format!(": {stderr}")
        } else {
            String::new()
        };

        Err(Error::new(
            ErrorKind::Other,
            format!(
                "`{}` reported non-zero exit-status{code}{stderr}",
                format_command(command, args)
            ),
        ))
    } else {
        Ok(())
    }
}

/// Compile `src` into `dst` using the provided compiler.
fn compile(compiler: &str, src: &Path, dst: &str, options: &[&str]) {
    let dst = src.with_file_name(dst);
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed={}", dst.display());

    let () = run(
        compiler,
        options
            .iter()
            .map(OsStr::new)
            .chain([src.as_os_str(), "-o".as_ref(), dst.as_os_str()]),
    )
    .unwrap_or_else(|err| panic!("failed to run `{compiler}`: {err}"));
}

/// Compile `src` into `dst` using `cc`.
#[cfg_attr(not(feature = "generate-c-header"), allow(dead_code))]
fn cc(src: &Path, dst: &str, options: &[&str]) {
    compile("cc", src, dst, options)
}

fn main() {
    #[cfg(feature = "generate-c-header")]
    {
        use std::fs::copy;
        use std::fs::write;

        let crate_dir = env!("CARGO_MANIFEST_DIR");

        cbindgen::Builder::new()
            .with_crate(crate_dir)
            .with_config(cbindgen::Config::from_root_or_default(crate_dir))
            .generate()
            .expect("Unable to generate bindings")
            .write_to_file(Path::new(crate_dir).join("include").join("blazesym.h"));

        // Generate a C program that just included blazesym.h as a basic
        // smoke test that cbindgen didn't screw up completely.
        let out_dir = env::var_os("OUT_DIR").unwrap();
        let out_dir = Path::new(&out_dir);
        let blaze_src_c = out_dir.join("blazesym.c");
        let () = write(
            &blaze_src_c,
            r#"
#include <blazesym.h>

int main() {
  return 0;
}
"#,
        )
        .unwrap();

        let blaze_src_cxx = out_dir.join("blazesym.cpp");
        let _bytes = copy(&blaze_src_c, &blaze_src_cxx).expect("failed to copy file");

        cc(
            &blaze_src_c,
            "blazesym.bin",
            &[
                "-Wall",
                "-Wextra",
                "-Werror",
                "-I",
                Path::new(crate_dir).join("include").to_str().unwrap(),
            ],
        );

        // Best-effort check that C++ can compile the thing as well. Hopefully
        // all flags are supported...
        for cxx in ["clang++", "g++"] {
            if which::which(cxx).is_ok() {
                compile(
                    cxx,
                    &blaze_src_cxx,
                    &format!("blazesym_cxx_{cxx}.bin"),
                    &[
                        "-Wall",
                        "-Wextra",
                        "-Werror",
                        "-I",
                        Path::new(crate_dir).join("include").to_str().unwrap(),
                    ],
                );
            }
        }
    }
}
