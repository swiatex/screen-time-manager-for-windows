use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Build the Windows resource (.rc) at compile time so the embedded
    // VERSIONINFO always matches the crate version from Cargo.toml. A binary
    // with proper version/company metadata looks far less suspicious to
    // antivirus heuristics than a stripped, metadata-less executable.
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or("0");
    let minor = parts.next().unwrap_or("0");
    let patch = parts.next().unwrap_or("0");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let resources = Path::new(&manifest_dir).join("resources");
    let out_dir = env::var("OUT_DIR").unwrap();

    // Copy the resource inputs into OUT_DIR before handing them to windres.
    // GNU windres receives paths in the ANSI code page, so a project directory
    // containing non-ASCII characters (e.g. Polish "ł") makes it fail with
    // "can't open icon file". OUT_DIR is ASCII-only.
    let ico_path = Path::new(&out_dir).join("app.ico");
    let manifest_path = Path::new(&out_dir).join("app.manifest");
    fs::copy(resources.join("app.ico"), &ico_path)
        .expect("failed to copy app.ico into OUT_DIR");
    fs::copy(resources.join("app.manifest"), &manifest_path)
        .expect("failed to copy app.manifest into OUT_DIR");

    // Backslashes in RC string literals are escape characters, so double them.
    let esc = |p: std::path::PathBuf| p.display().to_string().replace('\\', "\\\\");
    let ico = esc(ico_path);
    let manifest = esc(manifest_path);

    // NOTE: the VarFileInfo/"Translation" block is omitted on purpose. GNU
    // windres (MinGW) rejects it, and MSVC rc.exe works fine without it.
    let rc = format!(
        "1 ICON \"{ico}\"\n\
         1 24 \"{manifest}\"\n\
         1 VERSIONINFO\n\
         FILEVERSION {major},{minor},{patch},0\n\
         PRODUCTVERSION {major},{minor},{patch},0\n\
         FILEFLAGSMASK 0x3fL\n\
         FILEFLAGS 0x0L\n\
         FILEOS 0x40004L\n\
         FILETYPE 0x1L\n\
         FILESUBTYPE 0x0L\n\
         BEGIN\n\
         BLOCK \"StringFileInfo\"\n\
         BEGIN\n\
         BLOCK \"040904b0\"\n\
         BEGIN\n\
         VALUE \"CompanyName\", \"Shadow\"\n\
         VALUE \"FileDescription\", \"Screen Time Manager\"\n\
         VALUE \"FileVersion\", \"{version}.0\"\n\
         VALUE \"InternalName\", \"screen-time-manager\"\n\
         VALUE \"LegalCopyright\", \"Copyright (C) Shadow. MIT License.\"\n\
         VALUE \"OriginalFilename\", \"screen-time-manager.exe\"\n\
         VALUE \"ProductName\", \"Screen Time Manager\"\n\
         VALUE \"ProductVersion\", \"{version}.0\"\n\
         END\n\
         END\n\
         END\n",
    );

    let rc_path = Path::new(&out_dir).join("app.rc");
    fs::write(&rc_path, rc).expect("failed to write generated resource file");

    embed_resource::compile(&rc_path, embed_resource::NONE);

    println!("cargo:rerun-if-changed=resources/app.ico");
    println!("cargo:rerun-if-changed=resources/app.manifest");
    println!("cargo:rerun-if-changed=build.rs");
}
