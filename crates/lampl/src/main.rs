// SPDX-License-Identifier: Apache-2.0

fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--version") | Some("-V") => println!("lampl {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("usage: lampl --version");
            std::process::exit(2);
        }
    }
}
