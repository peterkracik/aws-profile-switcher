use dialoguer::{console::Term, theme::ColorfulTheme, Select};
use std::{process::Command, io::{self, Write}};


fn get_profiles() -> Vec<String> {
    let output = Command::new("aws")
        .arg("configure")
        .arg("list-profiles")
        .output()
        .expect("Failed to execute command");

    String::from_utf8(output.stdout).unwrap().lines().map(|s| s.to_string()).collect()
}

fn main() -> std::io::Result<()> {
    let items = get_profiles();
    let mut cmd = "export AWS_PROFILE=".to_string();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .items(&items)
        .default(0)
        .interact_on_opt(&Term::stderr())?;

    match selection {
        Some(index) => {
            cmd.push_str(&items[index]);
            io::stdout().write_all(cmd.as_bytes())?;
        }
        None => println!("User did not select anything")
    }

    Ok(())
}
