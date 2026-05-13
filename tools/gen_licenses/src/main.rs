use std::fs;
use std::path::Path;

fn main() {
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("licenses");

    fs::create_dir_all(&out_dir).expect("create licenses/");

    let acks = two_face::acknowledgement::listing();

    let mut md = String::from(
        "# Third-party syntax highlighting licenses\n\n\
         These licenses cover the syntax definitions bundled via the `two-face` crate\n\
         (curated by the [bat project](https://github.com/sharkdp/bat)).\n\n",
    );

    for license in acks.for_syntaxes() {
        license.write_md(&mut md);
    }

    let path = out_dir.join("SYNTAX_LICENSES.md");
    fs::write(&path, &md).expect("write SYNTAX_LICENSES.md");
    println!("wrote {}", path.display());
    println!("  {} syntax license entries", acks.for_syntaxes().len());
}
