//! Command-line grammar shared by the CLI, hotkey actions and schedule rules.
//!
//! ```text
//! Nitor.exe [--config <path>] [target] b <n | +n | -n> [[target] b ...]
//! Nitor.exe [target] color <#RRGGBB>
//! Nitor.exe list | get [target] | rescan | quit
//! target := alias | name-substring | serial | #index | all | cursor
//! ```

use crate::color::Rgb;
use std::path::PathBuf;

/// Which monitor(s) a command applies to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// No target given: the first monitor in tray order.
    Default,
    All,
    /// The monitor under the mouse pointer.
    Cursor,
    /// `#n`, 1-based as shown by `list`.
    Index(usize),
    /// Matched case-insensitively against alias, then friendly-name substring, then serial / stable ID.
    Name(String),
}

/// A brightness change, in percent (0–100).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrightnessOp {
    Set(u8),
    /// Relative change, may be negative.
    Add(i16),
    /// Switch to `b` if the current value equals `a`, otherwise to `a`.
    Toggle(u8, u8),
}

impl BrightnessOp {
    /// Apply to a current value, clamping into `min..=max`.
    pub fn apply(self, current: u8, min: u8, max: u8) -> u8 {
        let (lo, hi) = if min <= max { (min, max) } else { (0, 100) };
        let value = match self {
            BrightnessOp::Set(v) => i32::from(v),
            BrightnessOp::Add(d) => i32::from(current) + i32::from(d),
            BrightnessOp::Toggle(a, b) => {
                if current == a {
                    i32::from(b)
                } else {
                    i32::from(a)
                }
            }
        };
        value.clamp(i32::from(lo), i32::from(hi)) as u8
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Brightness { target: Target, op: BrightnessOp },
    Color { target: Target, color: Rgb },
    List,
    Get(Target),
    Rescan,
    Quit,
}

/// A fully parsed command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Invocation {
    pub config_path: Option<PathBuf>,
    pub commands: Vec<Command>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError(pub String);

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err(msg: impl Into<String>) -> CliError {
    CliError(msg.into())
}

/// Parse process arguments (without argv[0]).
pub fn parse(args: &[String]) -> Result<Invocation, CliError> {
    let mut inv = Invocation::default();
    let mut tokens: Vec<&str> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            if inv.config_path.is_some() {
                return Err(err("--config given more than once"));
            }
            let path = iter.next().ok_or_else(|| err("--config needs a path"))?;
            inv.config_path = Some(PathBuf::from(path));
        } else {
            tokens.push(arg.as_str());
        }
    }

    let mut i = 0;
    while i < tokens.len() {
        let (cmd, used) = parse_command(&tokens[i..])?;
        inv.commands.push(cmd);
        i += used;
    }
    Ok(inv)
}

/// Parse one command at the start of `toks`; returns it and the number of tokens used.
/// `toks` is never empty.
fn parse_command(toks: &[&str]) -> Result<(Command, usize), CliError> {
    let first = toks[0];
    match first.to_ascii_lowercase().as_str() {
        "list" => Ok((Command::List, 1)),
        "rescan" => Ok((Command::Rescan, 1)),
        "quit" => Ok((Command::Quit, 1)),
        "get" => match toks.get(1) {
            Some(t) if !is_keyword(t) => Ok((Command::Get(parse_target(t)?), 2)),
            _ => Ok((Command::Get(Target::Default), 1)),
        },
        "b" | "color" => parse_body(Target::Default, toks, 0),
        _ => match toks.get(1) {
            Some(kw) if is_brightness_or_color(kw) => {
                let target = parse_target(first)?;
                parse_body(target, toks, 1)
            }
            _ => Err(err(format!("unknown command '{first}'"))),
        },
    }
}

/// `b <value>` or `color <#RRGGBB>` starting at `toks[kw_idx]`.
fn parse_body(target: Target, toks: &[&str], kw_idx: usize) -> Result<(Command, usize), CliError> {
    let kw = toks[kw_idx];
    let value = toks
        .get(kw_idx + 1)
        .ok_or_else(|| err(format!("missing value after '{kw}'")))?;
    let used = kw_idx + 2;
    if kw.eq_ignore_ascii_case("color") {
        let color = Rgb::from_hex(value)
            .ok_or_else(|| err(format!("invalid color '{value}', expected #RRGGBB")))?;
        Ok((Command::Color { target, color }, used))
    } else {
        let op = parse_brightness_value(value)?;
        Ok((Command::Brightness { target, op }, used))
    }
}

fn is_keyword(tok: &str) -> bool {
    matches!(
        tok.to_ascii_lowercase().as_str(),
        "b" | "color" | "list" | "get" | "rescan" | "quit"
    )
}

fn is_brightness_or_color(tok: &str) -> bool {
    tok.eq_ignore_ascii_case("b") || tok.eq_ignore_ascii_case("color")
}

fn parse_target(tok: &str) -> Result<Target, CliError> {
    match tok.to_ascii_lowercase().as_str() {
        "all" => Ok(Target::All),
        "cursor" => Ok(Target::Cursor),
        _ => match tok.strip_prefix('#') {
            Some(rest) => match rest.parse::<usize>() {
                Ok(n) if n >= 1 => Ok(Target::Index(n)),
                _ => Err(err(format!("invalid monitor index '{tok}'"))),
            },
            None => Ok(Target::Name(tok.to_string())),
        },
    }
}

/// A brightness value: `n` (set), `+n` or `-n` (relative).
fn parse_brightness_value(tok: &str) -> Result<BrightnessOp, CliError> {
    if tok.starts_with('+') || tok.starts_with('-') {
        let n: i64 = tok
            .parse()
            .map_err(|_| err(format!("invalid brightness change '{tok}'")))?;
        Ok(BrightnessOp::Add(n.clamp(-100, 100) as i16))
    } else {
        Ok(BrightnessOp::Set(parse_percent(tok)?))
    }
}

/// An absolute percentage, clamped to 0..=100.
fn parse_percent(tok: &str) -> Result<u8, CliError> {
    if tok.starts_with('+') || tok.starts_with('-') {
        return Err(err(format!("invalid brightness '{tok}'")));
    }
    let n: i64 = tok
        .parse()
        .map_err(|_| err(format!("invalid brightness '{tok}'")))?;
    Ok(n.clamp(0, 100) as u8)
}

/// `a/b` for `toggle`.
fn parse_toggle(tok: &str) -> Result<BrightnessOp, CliError> {
    let (a, b) = tok
        .split_once('/')
        .ok_or_else(|| err(format!("expected toggle a/b, got '{tok}'")))?;
    Ok(BrightnessOp::Toggle(parse_percent(a)?, parse_percent(b)?))
}

/// Whether a token begins a bare action, meaning no target precedes it.
fn starts_action(tok: &str) -> bool {
    let lower = tok.to_ascii_lowercase();
    matches!(lower.as_str(), "set" | "b" | "toggle")
        || tok.starts_with(['+', '-'])
        || tok.starts_with(|c: char| c.is_ascii_digit())
}

/// Parse a hotkey action or schedule rule value, e.g. `all +10`, `cursor set 80`,
/// `Dell toggle 30/80`, `all b 40`, `-5`.
pub fn parse_action(s: &str) -> Result<(Target, BrightnessOp), CliError> {
    let toks: Vec<&str> = s.split_whitespace().collect();
    let Some(&first) = toks.first() else {
        return Err(err("empty action"));
    };
    let (target, rest) = if starts_action(first) {
        (Target::Default, &toks[..])
    } else {
        (parse_target(first)?, &toks[1..])
    };
    let op = match rest {
        [] => return Err(err(format!("missing brightness in '{s}'"))),
        [kw, v] if kw.eq_ignore_ascii_case("set") => BrightnessOp::Set(parse_percent(v)?),
        [kw, v] if kw.eq_ignore_ascii_case("b") => parse_brightness_value(v)?,
        [kw, v] if kw.eq_ignore_ascii_case("toggle") => parse_toggle(v)?,
        [v] => parse_brightness_value(v)?,
        _ => return Err(err(format!("invalid action '{s}'"))),
    };
    Ok((target, op))
}

/// Usage text printed for `--help` or a parse error.
pub const USAGE: &str = "\
Usage: Nitor.exe [--config <path>] [command...]
  [target] b <n|+n|-n>     set or change brightness (repeatable)
  [target] color <#RRGGBB> set a monitor's tray color
  list                     list monitors
  get [target]             print brightness (also the exit code)
  rescan                   re-detect monitors
  quit                     exit the running instance
target: alias | name-substring | serial | #index | all | cursor";

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn brightness(target: Target, op: BrightnessOp) -> Command {
        Command::Brightness { target, op }
    }

    #[test]
    fn empty_args_have_no_commands() {
        let inv = parse(&[]).unwrap();
        assert_eq!(inv, Invocation::default());
    }

    #[test]
    fn repeated_brightness_commands() {
        let inv = parse(&args("Dell b +10 LG b 30")).unwrap();
        assert_eq!(
            inv.commands,
            vec![
                brightness(Target::Name("Dell".into()), BrightnessOp::Add(10)),
                brightness(Target::Name("LG".into()), BrightnessOp::Set(30)),
            ]
        );
    }

    #[test]
    fn default_target_and_negative_add() {
        let inv = parse(&args("b -20")).unwrap();
        assert_eq!(
            inv.commands,
            vec![brightness(Target::Default, BrightnessOp::Add(-20))]
        );
    }

    #[test]
    fn config_anywhere_once() {
        let inv = parse(&args("b 50 --config x.ini list")).unwrap();
        assert_eq!(inv.config_path, Some(PathBuf::from("x.ini")));
        assert_eq!(inv.commands.len(), 2);
        assert!(parse(&args("--config a.ini --config b.ini")).is_err());
        assert!(parse(&args("list --config")).is_err());
    }

    #[test]
    fn simple_commands_case_insensitive() {
        let inv = parse(&args("LIST Rescan quit")).unwrap();
        assert_eq!(
            inv.commands,
            vec![Command::List, Command::Rescan, Command::Quit]
        );
        let inv = parse(&args("Get Cursor get")).unwrap();
        assert_eq!(
            inv.commands,
            vec![Command::Get(Target::Cursor), Command::Get(Target::Default)]
        );
    }

    #[test]
    fn targets() {
        let inv = parse(&args("all b 10 cursor b 20 #2 b 30 ALL b 40")).unwrap();
        let targets: Vec<Target> = inv
            .commands
            .iter()
            .map(|c| match c {
                Command::Brightness { target, .. } => target.clone(),
                _ => panic!("expected brightness"),
            })
            .collect();
        assert_eq!(
            targets,
            vec![Target::All, Target::Cursor, Target::Index(2), Target::All]
        );
        assert!(parse(&args("#0 b 10")).is_err());
        assert!(parse(&args("#x b 10")).is_err());
    }

    #[test]
    fn set_is_clamped_and_validated() {
        let inv = parse(&args("b 150 b 0")).unwrap();
        assert_eq!(
            inv.commands,
            vec![
                brightness(Target::Default, BrightnessOp::Set(100)),
                brightness(Target::Default, BrightnessOp::Set(0)),
            ]
        );
        assert!(parse(&args("b abc")).is_err());
        assert!(parse(&args("b")).is_err());
        assert!(parse(&args("all")).is_err());
    }

    #[test]
    fn unknown_input_is_an_error() {
        assert!(parse(&args("frobnicate")).is_err());
        assert!(parse(&args("-h")).is_err());
        assert!(parse(&args("Dell")).is_err());
    }

    #[test]
    fn color_command() {
        let inv = parse(&args("Dell color #1A2B3C")).unwrap();
        assert_eq!(
            inv.commands,
            vec![Command::Color {
                target: Target::Name("Dell".into()),
                color: Rgb::new(0x1A, 0x2B, 0x3C),
            }]
        );
        assert!(parse(&args("color #zz0000")).is_err());
    }

    #[test]
    fn apply_set_add_toggle() {
        assert_eq!(BrightnessOp::Set(50).apply(10, 0, 100), 50);
        assert_eq!(BrightnessOp::Set(0).apply(10, 10, 100), 10);
        assert_eq!(BrightnessOp::Add(10).apply(95, 0, 100), 100);
        assert_eq!(BrightnessOp::Add(-20).apply(10, 5, 100), 5);
        assert_eq!(BrightnessOp::Add(-5).apply(50, 0, 100), 45);
        assert_eq!(BrightnessOp::Toggle(30, 80).apply(30, 0, 100), 80);
        assert_eq!(BrightnessOp::Toggle(30, 80).apply(50, 0, 100), 30);
        assert_eq!(BrightnessOp::Toggle(30, 80).apply(80, 0, 100), 30);
        // Bad bounds fall back to 0..=100.
        assert_eq!(BrightnessOp::Set(70).apply(0, 90, 10), 70);
    }

    #[test]
    fn parse_action_forms() {
        assert_eq!(
            parse_action("all +10").unwrap(),
            (Target::All, BrightnessOp::Add(10))
        );
        assert_eq!(
            parse_action("cursor set 80").unwrap(),
            (Target::Cursor, BrightnessOp::Set(80))
        );
        assert_eq!(
            parse_action("Dell toggle 30/80").unwrap(),
            (Target::Name("Dell".into()), BrightnessOp::Toggle(30, 80))
        );
        assert_eq!(
            parse_action("all b 40").unwrap(),
            (Target::All, BrightnessOp::Set(40))
        );
        assert_eq!(
            parse_action("-5").unwrap(),
            (Target::Default, BrightnessOp::Add(-5))
        );
        assert_eq!(
            parse_action("5").unwrap(),
            (Target::Default, BrightnessOp::Set(5))
        );
        assert_eq!(
            parse_action("#3 +5").unwrap(),
            (Target::Index(3), BrightnessOp::Add(5))
        );
        assert_eq!(
            parse_action("ALL SET 200").unwrap(),
            (Target::All, BrightnessOp::Set(100))
        );
    }

    #[test]
    fn parse_action_errors() {
        assert!(parse_action("").is_err());
        assert!(parse_action("   ").is_err());
        assert!(parse_action("all").is_err());
        assert!(parse_action("all toggle 30").is_err());
        assert!(parse_action("all +").is_err());
        assert!(parse_action("all set").is_err());
        assert!(parse_action("#0 +5").is_err());
    }
}
