use std::fmt::Display;
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::thread;

use tbp::frontend_msg;
use wait_timeout::ChildExt;

pub struct BotInstance {
    command: Command,
    state: Option<State>,
}

#[derive(Debug)]
pub enum BotError {
    NoBot,
    Exited(ExitStatus),
}

struct State {
    child: Child,
    to_bot: ChildStdin,
    from_bot: Receiver<tbp::BotMessage>,
}

impl BotInstance {
    pub fn new(path: &Path) -> Self {
        let mut command = Command::new(path);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        BotInstance {
            command,
            state: None,
        }
    }

    pub fn launch(&mut self) -> anyhow::Result<tbp::bot_msg::Info> {
        let _ = self.send_message(frontend_msg::Quit::default());
        self.state = None;
        let mut child = self.command.spawn()?;

        let (send, from_bot) = channel();
        let bot_stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        thread::spawn(move || {
            for line in bot_stdout.lines() {
                let line = match line {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let tbp::MaybeUnknown::Known(msg) = value {
                    if send.send(msg).is_err() {
                        break;
                    }
                }
            }
        });

        self.state = Some(State {
            to_bot: child.stdin.take().unwrap(),
            from_bot,
            child,
        });

        match self.block_message()? {
            tbp::BotMessage::Info(info) => Ok(info),
            _ => Err(anyhow::anyhow!("Expected `info` to be the first message")),
        }
    }

    pub fn poll_message(&mut self) -> Result<Option<tbp::BotMessage>, BotError> {
        let state = self.check_state()?;
        match state.from_bot.try_recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(e) => {
                self.check()?;
                panic!("Bot process is fine, but got error: {}", e);
            }
        }
    }

    pub fn block_message(&mut self) -> Result<tbp::BotMessage, BotError> {
        let state = self.check_state()?;
        match state.from_bot.recv() {
            Ok(msg) => Ok(msg),
            Err(e) => {
                self.check()?;
                panic!("Bot process is fine, but got error: {}", e);
            }
        }
    }

    pub fn send_message(&mut self, msg: impl Into<tbp::FrontendMessage>) -> Result<(), BotError> {
        let state = self.check_state()?;
        let mut msg = serde_json::to_string(&msg.into()).unwrap();
        msg.push('\n');
        match state.to_bot.write_all(msg.as_bytes()) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.check()?;
                panic!("Bot process is fine, but got error: {}", e);
            }
        }
    }

    pub fn check(&mut self) -> Result<(), BotError> {
        self.check_state().map(|_| ())
    }

    fn check_state(&mut self) -> Result<&mut State, BotError> {
        let state = self.state.as_mut().ok_or(BotError::NoBot)?;
        match state.child.try_wait().unwrap() {
            Some(status) => Err(BotError::Exited(status)),
            None => Ok(state),
        }
    }
}

impl Drop for BotInstance {
    fn drop(&mut self) {
        let _ = self.send_message(frontend_msg::Quit::default());
        if let Some(mut state) = self.state.take() {
            drop(state.to_bot);
            match state
                .child
                .wait_timeout(std::time::Duration::from_millis(50))
            {
                Ok(None) => {
                    // Make sure process exits
                    let _ = state.child.kill();
                    let _ = state.child.wait();
                }
                _ => {}
            }
        }
    }
}

impl Display for BotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BotError::NoBot => write!(f, "no bot has been launched"),
            BotError::Exited(status) => write!(f, "the bot exited: {}", status),
        }
    }
}

impl std::error::Error for BotError {}
