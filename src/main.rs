use my_app::{parse_app_args, render_app_output_json, run_workload, ParseAppArgsError};

fn main() {
    let workload = match parse_app_args(std::env::args().skip(1)) {
        Ok(workload) => workload,
        Err(ParseAppArgsError::HelpRequested) => {
            println!("{}", my_app::app_usage());
            return;
        }
        Err(ParseAppArgsError::Message(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    let output = run_workload(&workload);
    println!("{}", render_app_output_json(&output));
}
