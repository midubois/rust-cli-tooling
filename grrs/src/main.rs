use clap::Parser;


/// Command line arguments for the grrs tool.
#[derive(Parser)]
struct Cli {
    pattern: String,
    path: std::path::PathBuf,
}

fn main() {
    let args = Cli::parse();
    //let pattern = std::env::args().nth(1).expect("No pattern given");
    //let path = std::env::args().nth(2).expect("No path given");

    // let args = Cli {
    //     pattern,
    //     path: std::path::PathBuf::from(path),
    // };

    println!("pattern: {:?}, path: {:?}", args.pattern, args.path);
}