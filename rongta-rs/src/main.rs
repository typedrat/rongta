use usage::Cli;

/// Experimental Rongta R22 printer protocol tools
///
/// Printer commands are not implemented yet.
#[derive(Cli)]
#[usage(bin = "rongta", version = env!("CARGO_PKG_VERSION"))]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
