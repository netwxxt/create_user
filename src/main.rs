use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn prompt(label: &str) -> String {
    print!("{label}: ");
    io::stdout().flush().unwrap();
    let mut s = String::new();
    io::stdin().read_line(&mut s).unwrap();
    s.trim().to_string()
}

fn run(cmd: &str, args: &[&str]) -> io::Result<()> {
    let status = Command::new(cmd).args(args).status()?;
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("{cmd} {args:?} failed: {status}"),
        ));
    }
    Ok(())
}

fn main() -> io::Result<()> {
    if unsafe { libc_geteuid() } != 0 {
        eprintln!("must run as root");
        std::process::exit(1);
    }

    let username = prompt("username");
    let pubkey = prompt("ssh public key (paste full line)");

    // create user with home dir + bash shell
    run("useradd", &["-m", "-s", "/bin/bash", &username])?;

    // ensure sudo installed
    let sudo_check = Command::new("sh").args(["-c", "command -v sudo"]).output()?;
    if String::from_utf8_lossy(&sudo_check.stdout).trim().is_empty() {
        Command::new("apt").arg("update").env("DEBIAN_FRONTEND", "noninteractive").status()?;
        Command::new("apt")
            .args(["install", "-y", "sudo"])
            .env("DEBIAN_FRONTEND", "noninteractive")
            .status()?;
    }

    // lock password (no password login, key-only)
    run("passwd", &["-d", &username])?;

    // sudo group
    run("usermod", &["-aG", "sudo", &username])?;

    // passwordless sudo drop-in
    fs::create_dir_all("/etc/sudoers.d")?;
    let sudoers_path = format!("/etc/sudoers.d/{username}");
    fs::write(&sudoers_path, format!("{username} ALL=(ALL) NOPASSWD:ALL\n"))?;
    fs::set_permissions(&sudoers_path, fs::Permissions::from_mode(0o440))?;

    // ssh dir + authorized_keys
    let home = format!("/home/{username}");
    let ssh_dir = format!("{home}/.ssh");
    fs::create_dir_all(&ssh_dir)?;
    fs::write(format!("{ssh_dir}/authorized_keys"), format!("{pubkey}\n"))?;
    fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700))?;
    fs::set_permissions(format!("{ssh_dir}/authorized_keys"), fs::Permissions::from_mode(0o600))?;

    run("chown", &["-R", &format!("{username}:{username}"), &ssh_dir])?;

    println!("done: {username} created, sudo NOPASSWD set, ssh key installed");

    let do_shell = prompt("configure fish + starship for this user? [Y/n]");
    if !do_shell.eq_ignore_ascii_case("n") {
        setup_shell(&username, &home)?;
    }

    Ok(())
}

fn setup_shell(username: &str, home: &str) -> io::Result<()> {
    let is_main = prompt("is this the main device? [Y/n]");
    let (arrow_style, user_style) = if is_main.eq_ignore_ascii_case("n") {
        ("bold yellow", "bold purple")
    } else {
        ("bold green", "bold cyan")
    };

    Command::new("apt").arg("update").env("DEBIAN_FRONTEND", "noninteractive").status()?;
    Command::new("apt")
        .args(["install", "-y", "fish", "curl"])
        .env("DEBIAN_FRONTEND", "noninteractive")
        .status()?;

    let which_out = Command::new("sh").args(["-c", "command -v fish"]).output()?;
    let fish_path = String::from_utf8(which_out.stdout)
        .unwrap_or_default()
        .trim()
        .to_string();
    if fish_path.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "fish not found after install"));
    }

    let shells = fs::read_to_string("/etc/shells").unwrap_or_default();
    if !shells.contains(&fish_path) {
        let mut f = fs::OpenOptions::new().append(true).open("/etc/shells")?;
        writeln!(f, "{fish_path}")?;
    }

    run("chsh", &["-s", &fish_path, username])?;

    Command::new("sh")
        .arg("-c")
        .arg("curl -sS https://starship.rs/install.sh | sh -s -- -y")
        .status()?;

    let fish_dir = format!("{home}/.config/fish");
    let completions_dir = format!("{fish_dir}/completions");
    fs::create_dir_all(&completions_dir)?;

    let config_fish = r#"if status is-interactive
    set -g FISH_COMPLETIONS $HOME/.config/fish/completions

    set -gx CDPATH ".:$HOME/projects"

    function fish_greeting
        fish -N --version
    end

    function fish_title
        echo "$USER@$(string split . (hostname))[1]:$(basename (pwd))"
    end
end

alias ll 'ls -al'
alias sc systemctl
alias jc journalctl

fish_add_path -P "$HOME/.cargo/bin"
if not test -e $FISH_COMPLETIONS/cargo.fish
    mkdir -p $FISH_COMPLETIONS
    curl -s https://raw.githubusercontent.com/fish-shell/fish-shell/master/share/completions/cargo.fish \
        -o $FISH_COMPLETIONS/cargo.fish
end

starship init fish | source
"#;
    fs::write(format!("{fish_dir}/config.fish"), config_fish)?;

    let starship_toml = format!(
        r#"format = """
[╭─]({arrow_style})$username$hostname$directory$git_branch$git_status$rust$python
[╰─❯]({arrow_style}) """

[username]
style_user = "{user_style}"
show_always = true

[hostname]
ssh_only = false
format = "[@$hostname](bold blue) "

[directory]
truncation_length = 3
style = "bold yellow"

[git_branch]
symbol = " "
style = "bold purple"

[git_status]
style = "bold red"

[rust]
symbol = "🦀 "

[python]
symbol = "🐍 "
"#
    );
    fs::write(format!("{home}/.config/starship.toml"), starship_toml)?;

    run("chown", &["-R", &format!("{username}:{username}"), &format!("{home}/.config")])?;

    println!("shell configured for {username}: fish + starship, log out/in to apply");
    Ok(())
}

unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}