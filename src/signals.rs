//! Unix signals other tools can use to get the always-on-top panel out of the way:
//!
//! ```text
//! pkill -USR1 -x quick-note   # hide the window
//! pkill -USR2 -x quick-note   # show it again
//! ```
//!
//! A fullscreen overlay (screenshot tools, screen recorders) otherwise ends up
//! below our always-on-top window and stops receiving clicks and keys.

use std::sync::mpsc::{self, Receiver};
use std::thread;

use signal_hook::consts::{SIGUSR1, SIGUSR2};
use signal_hook::iterator::Signals;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalCommand {
    Hide,
    Show,
}

/// Starts a listener thread; `ctx` is woken so the app reacts immediately.
pub fn listen(ctx: egui::Context) -> Receiver<SignalCommand> {
    let (tx, rx) = mpsc::channel();
    match Signals::new([SIGUSR1, SIGUSR2]) {
        Ok(mut signals) => {
            thread::Builder::new()
                .name("signals".into())
                .spawn(move || {
                    for signal in signals.forever() {
                        let cmd = match signal {
                            SIGUSR1 => SignalCommand::Hide,
                            SIGUSR2 => SignalCommand::Show,
                            _ => continue,
                        };
                        if tx.send(cmd).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                })
                .ok();
        }
        Err(e) => eprintln!("quick-note: không đăng ký được tín hiệu: {e}"),
    }
    rx
}
