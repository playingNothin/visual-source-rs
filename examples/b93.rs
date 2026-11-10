//! stdin (binary) -> stdout (base93 text). Uses the crate's proven encoder.
use std::io::{Read, Write};

fn main() {
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf).expect("read stdin");
    print!("{}", visual_source_rs::base93::encode(&buf));
    let _ = std::io::stdout().flush();
}
