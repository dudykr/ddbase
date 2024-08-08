use clap::Parser;

#[derive(Debug, Parser)]
struct CliArgs {}

fn main() {
    let args = CliArgs::parse();
}
