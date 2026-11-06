//! The moment the sign-in card fills a page, and the two small browser questions around it (which page is in front, sign
//! out of a site). Only the app's own commands reach these; the bot has no tool that does.
//!
//! The order for a fill, and why each step is where it is:
//!
//! 1. every call of the bot is refused (`login_busy`), and the one-call-at-a-time lock is taken, so a call that was
//!    already running finishes first and none starts until the fill is over. The bot cannot look at the screen, run a
//!    command or use the page tool to read the box or to swap the page under the fill;
//! 2. the screen lease is held for the bot's own run (renewed if it held it, taken if it was free, refused if the
//!    person or another bot has it). The person holding the screen can type the password themselves;
//! 3. the front tab's origin is read from the browser and compared here with the card's site, before the password
//!    leaves the host. A page that is not that exact https origin gets nothing;
//! 4. the password goes on the page tool's stdin (`Sandbox::browser_login`), which checks the origin again inside the
//!    page in the same evaluation as the fill, fills password boxes only, presses Enter and reads back that no box
//!    still holds it;
//! 5. the lease is given back if the fill took it, and the bot's calls are let in again, whatever happened (both are
//!    dropped guards, so even a crash in the page tool lets go).
//!
//! The page tool waits up to about 15 seconds after Enter for the page to show the result (a login page takes a moment), so
//! steps 1 to 5 can last that long; the bot stays blocked and the screen stays held until it answers.
//!
//! What this cannot do: a program the bot started earlier as user `pm` shares a user with Chrome and its debugging port,
//! so it could watch the page during these few seconds. Refusing the bot's new calls does not stop a program already
//! running. The card says so and the owner can use Take over instead.

use std::time::Duration;

use super::login::{self, Done, Request};
use super::sandbox::browser;
use super::sandbox::lease::{Holder, LeaseError};
use super::sandbox_calls::{run_id, CallError, Calls, Port};

/// The longest a fill waits for a call of the bot that is already running.
const WAIT_FOR_RUNNING_CALL: Duration = Duration::from_secs(30);
/// The longest a question about the front page waits for the screen's one-at-a-time lock.
const WAIT_FOR_QUESTION: Duration = Duration::from_secs(3);

/// Lets the bot's calls in again when it is dropped, however the fill ended.
struct Busy<'a>(&'a std::sync::atomic::AtomicBool);

impl<'a> Busy<'a> {
    fn enter(flag: &'a std::sync::atomic::AtomicBool) -> Result<Busy<'a>, CallError> {
        if flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(CallError::new("LOGIN_IN_PROGRESS", "A sign-in is already being filled. Wait a few seconds."));
        }
        Ok(Busy(flag))
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Gives the screen back when it is dropped, however the fill ended (an answer, an error or a crash in the page tool).
struct Lent<'a> {
    lease: &'a parking_lot::Mutex<super::sandbox::lease::ScreenLease>,
    id: Option<String>,
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.lease.lock().release(&id);
        }
    }
}

fn from_login(error: login::LoginError) -> CallError {
    CallError::new(error.code, error.message)
}

impl Calls {
    /// The origin of the browser's front tab, read from the browser. `None`: no page could be read (no tab is open,
    /// the answer was not understood). The caller checks the computer is running first: this would start a stopped one.
    pub fn front_origin(&self, port: &Port) -> Option<String> {
        let _one = self.serial.try_lock_for(WAIT_FOR_QUESTION)?;
        let raw = port.sandbox.browser(&browser::origin_args()).ok()?;
        login::parse_origin_reply(&raw)
    }

    /// Fills the sign-in form in the browser's front tab and submits it. See the file header.
    pub fn login_fill(&self, port: &Port, bot: &str, conversation: &str, request: Request) -> Result<Done, CallError> {
        let _busy = Busy::enter(&self.login_busy)?;
        let Some(_one) = self.serial.try_lock_for(WAIT_FOR_RUNNING_CALL) else {
            return Err(CallError::new("LOGIN_SCREEN_BUSY", "The bot is in the middle of something on the computer. Try Sign in again in a moment."));
        };

        let run = run_id(bot, conversation);
        let now = self.host.now_ms();
        let (lease_id, took_it) = {
            let mut lease = port.lease.lock();
            let had = lease.current(now).is_some_and(|l| matches!(&l.holder, Holder::Bot(b) if b == bot) && l.run_id == run);
            match lease.take_for_bot(bot, &run, now) {
                Ok(id) => (id, !had),
                Err(LeaseError::PersonHasControl) => {
                    return Err(CallError::new("LOGIN_PERSON_HAS_SCREEN", "You have the screen right now. Type the password there yourself, or hand the screen back and press Sign in again."));
                }
                Err(LeaseError::BusyWithBot(_)) => {
                    return Err(CallError::new("LOGIN_SCREEN_BUSY", "Another bot is using the computer right now. Try Sign in again in a moment."));
                }
                Err(_) => return Err(CallError::new("LOGIN_SCREEN_BUSY", "The screen could not be taken. Try Sign in again in a moment.")),
            }
        };

        let _lent = Lent { lease: &port.lease, id: took_it.then_some(lease_id) };
        self.fill_held(port, request)
    }

    fn fill_held(&self, port: &Port, request: Request) -> Result<Done, CallError> {
        // The host reads the real origin of the front tab, and compares it itself, before the password goes anywhere.
        let actual = port.sandbox.browser(&browser::origin_args()).ok().and_then(|raw| login::parse_origin_reply(&raw));
        login::check_front(&request.origin, actual.as_deref()).map_err(from_login)?;

        let reply = port.sandbox.browser_login(request.stdin_bytes());
        // `request` (and the password in it) is dropped, and wiped, at the end of this function.
        let raw = reply.map_err(CallError::from)?;
        login::parse_login_reply(&raw).map_err(from_login)
    }

    /// Signs out of a site: its cookies and stored data go from the browser. `origin` is a card's normalized https origin.
    pub fn forget_site(&self, port: &Port, origin: &str) -> Result<u64, CallError> {
        let Some(_one) = self.serial.try_lock_for(WAIT_FOR_RUNNING_CALL) else {
            return Err(CallError::new("LOGIN_SCREEN_BUSY", "The bot is in the middle of something on the computer. Try again in a moment."));
        };
        let args = browser::forget_args(origin).map_err(CallError::from)?;
        let raw = port.sandbox.browser(&args)?;
        login::parse_forget_reply(&raw).map_err(from_login)
    }
}
