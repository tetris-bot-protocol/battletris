use std::fmt::Write as FmtWrite;
use std::io::{stdout, Write};
use std::ops::RangeInclusive;
use std::path::PathBuf;

use battle::BattleConfig;
use structopt::StructOpt;
use tbp::randomizer::RandomizerRule;
use tbp::{bot_msg, frontend_msg};

use crate::battle::Side;
use crate::bot::BotInstance;

mod battle;
mod bot;

#[derive(StructOpt)]
struct Options {
    bot_a: PathBuf,
    bot_b: PathBuf,

    #[structopt(short, long)]
    quiet: bool,

    #[structopt(short, long)]
    format: MatchFormat,

    #[structopt(long)]
    config: BattleConfig,
}

fn main() {
    match run(Options::from_args()) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{}", e);
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum MatchFormat {
    FirstTo(u32),
    Count(u32),
    Sprt(f64, f64),
}

impl MatchFormat {
    fn should_continue(self, w: u32, l: u32) -> bool {
        match self {
            MatchFormat::Count(c) => w + l < c,
            MatchFormat::FirstTo(c) => w != c && l != c,
            MatchFormat::Sprt(elo0, elo1) => {
                sprt_bounds(0.05, 0.05).contains(&llr(w, l, elo0, elo1))
            }
        }
    }

    fn extra_info(self, w: u32, l: u32, buf: &mut String) {
        match self {
            MatchFormat::Count(_) => {}
            MatchFormat::FirstTo(_) => {}
            MatchFormat::Sprt(elo0, elo1) => {
                let bounds = sprt_bounds(0.05, 0.05);
                write!(
                    buf,
                    "LLR: {:.2} ({:.2}, {:.2})",
                    llr(w, l, elo0, elo1),
                    bounds.start(),
                    bounds.end()
                )
                .unwrap();
            }
        }
    }
}

fn llr(w: u32, l: u32, elo0: f64, elo1: f64) -> f64 {
    if w == 0 || l == 0 {
        return 0.0;
    }

    let n = (w + l) as f64;
    let mean = w as f64 / n;
    let var_s = (mean - mean * mean) / n;

    let p0 = 1.0 / (1.0 + 10.0f64.powf(elo0 / 400.0));
    let p1 = 1.0 / (1.0 + 10.0f64.powf(elo1 / 400.0));

    (p1 - p0) * (2.0 * mean - p0 - p1) / var_s / 2.0
}

fn sprt_bounds(alpha: f64, beta: f64) -> RangeInclusive<f64> {
    let lower = (beta / (1.0 - alpha)).ln();
    let upper = ((1.0 - beta) / alpha).ln();
    lower..=upper
}

impl std::str::FromStr for MatchFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let s = s.to_lowercase();
        if let Some(rest) = s.strip_prefix("ft") {
            Ok(MatchFormat::FirstTo(rest.parse()?))
        } else if let Some(rest) = s.strip_prefix("sprt") {
            if rest.is_empty() {
                Ok(MatchFormat::Sprt(0.0, 5.0))
            } else {
                let (elo0, elo1) = rest
                    .strip_prefix("[")
                    .and_then(|s| s.strip_suffix("]"))
                    .and_then(|s| s.split_once(","))
                    .ok_or(anyhow::anyhow!("failed to parse sprt parameters"))?;
                Ok(MatchFormat::Sprt(
                    elo0.trim().parse()?,
                    elo1.trim().parse()?,
                ))
            }
        } else {
            Ok(MatchFormat::Count(s.parse()?))
        }
    }
}

impl std::fmt::Display for MatchFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchFormat::Count(c) => write!(f, "{} games", c),
            MatchFormat::FirstTo(c) => write!(f, "FT{}", c),
            MatchFormat::Sprt(elo0, elo1) => write!(f, "SPRT [{}, {}]", elo0, elo1),
        }
    }
}

fn run(options: Options) -> anyhow::Result<()> {
    let mut left = BotInstance::new(&options.bot_a.canonicalize()?);
    let mut right = BotInstance::new(&options.bot_b.canonicalize()?);

    let left_info = left.launch()?;
    let right_info = right.launch()?;

    if !options.quiet {
        println!(
            "{} {} VS {} {} ({})",
            left_info.name, left_info.version, right_info.name, right_info.version, options.format
        );
    }

    let mut left_wins = 0;
    let mut right_wins = 0;
    let mut left_crashes = 0;
    let mut right_crashes = 0;

    while options.format.should_continue(left_wins, right_wins) {
        if !options.quiet {
            let mut result = String::new();
            write!(&mut result, "{} ", left_wins).unwrap();
            if left_crashes != 0 {
                write!(&mut result, "({} crashes) ", left_crashes).unwrap();
            }
            write!(&mut result, "- {}", right_wins).unwrap();
            if right_crashes != 0 {
                write!(&mut result, " ({} crashes)", right_crashes).unwrap();
            }
            while result.len() < 30 {
                result.push(' ');
            }
            options
                .format
                .extra_info(left_wins, right_wins, &mut result);
            print!("\r\x1B[K{}", result);
            let _ = stdout().flush();
        }

        match battle::battle(&mut left, &mut right, &options.config) {
            Side::Left => left_wins += 1,
            Side::Right => right_wins += 1,
        }

        let _ = left.send_message(tbp::frontend_msg::Stop::new());
        let _ = right.send_message(tbp::frontend_msg::Stop::new());

        if left.check().is_err() {
            left_crashes += 1;
            load_bot(&mut left)?;
        }
        if right.check().is_err() {
            right_crashes += 1;
            load_bot(&mut right)?;
        }
    }

    if !options.quiet {
        print!("\r\x1B[K");
    }
    println!("{} - {}", left_wins, right_wins);
    println!("Crashes: {} - {}", left_crashes, right_crashes);
    if !options.quiet {
        let mut result = String::new();
        options
            .format
            .extra_info(left_wins, right_wins, &mut result);
        if !result.is_empty() {
            println!("{}", result);
        }
    }

    Ok(())
}

fn load_bot(bot: &mut BotInstance) -> anyhow::Result<bot_msg::Info> {
    bot.launch()?;
    let info = match bot.block_message()? {
        tbp::BotMessage::Info(info) => info,
        _ => anyhow::bail!("Expected info message upon startup"),
    };
    let mut rules = frontend_msg::Rules::new();
    rules.randomizer = RandomizerRule::SevenBag;
    bot.send_message(rules)?;
    match bot.block_message()? {
        tbp::BotMessage::Error(_) => anyhow::bail!("bot does not support these rules"),
        tbp::BotMessage::Ready(_) => {}
        _ => anyhow::bail!("Expected ready or error after rules message"),
    }
    Ok(info)
}