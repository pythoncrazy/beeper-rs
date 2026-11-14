fn run(args: &[&str]) -> (Option<i32>, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_beeper-rs"))
        .args(args)
        .env_remove("BEEPER_PAS_HOST")
        .env_remove("BEEPER_PAS_PORT")
        .output()
        .expect("run beeper-rs");
    let text = |b: Vec<u8>| String::from_utf8(b).expect("UTF-8 output");
    (out.status.code(), text(out.stdout), text(out.stderr))
}

#[test]
fn bad_command_lines_print_the_usage_and_exit_with_the_oclif_codes() {
    let usage = "See more help with --help\n\nUSAGE\n  $ beeper-rs [COMMAND] [FLAGS] [LINK]\n";
    for (args, error, code) in [
        (&["--bogus"][..], "Nonexistent flag: --bogus", 2),
        (&["--port", "abc"], "Parsing --port: Expected an integer from 1 to 65535 but received: abc", 2),
        (&["--data-dir"], "Flag --data-dir expects a value", 2),
        (&["chatz"], "chatz is not a beeper-rs command.", 127),
    ] {
        assert_eq!(run(args), (Some(code), String::new(), format!("Error: {error}\n{usage}")), "{args:?}");
    }
    let (code, help, _) = run(&["help"]);
    assert!(code == Some(0) && help.starts_with("Beeper chat app"), "{help}");
    let (code, version, _) = run(&["-V"]);
    assert!(code == Some(0) && version.starts_with("beeper-rs/"), "{version}");
}
