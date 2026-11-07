//! The one place where Potato Bot and Spaces meet. It holds a weak hold on each side, so neither keeps the other alive.
//!
//! Spaces hands a line to `BotSink::deliver`; Potato's answer comes back through `SpacePort`. Both are connected only
//! while the Spaces service is running (Labs `spaces_enabled` on) and the Potato service exists. With the switch off
//! nothing is connected, so no line reaches a bot and no answer can be posted.
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use super::service::{Service, SpacePort};
use crate::spaces::bot_bridge::{BotSink, Candidate, Delivery, Refusal};
use crate::spaces::diary::DiaryHome;
use crate::spaces::service::SpacesService;

struct Link {
    potato: Weak<Service>,
    spaces: Weak<SpacesService>,
}

impl BotSink for Link {
    fn deliver(&self, d: &Delivery) -> Result<(), Refusal> {
        self.potato.upgrade().ok_or(Refusal::Unavailable)?.space_deliver(d)
    }
    fn candidates(&self) -> Vec<Candidate> {
        self.potato.upgrade().map(|p| p.space_candidates()).unwrap_or_default()
    }
}

impl DiaryHome for Link {
    fn vault_of(&self, bot: &str) -> Option<PathBuf> {
        self.potato.upgrade()?.space_vault(bot)
    }
}

impl SpacePort for Link {
    fn reply(&self, bot: &str, delivery: &str, conversation: &str, text: &str) {
        if let Some(spaces) = self.spaces.upgrade() {
            // A held-back or refused answer is not an error to log with its text; the Space keeps its own counter.
            if let Err(why) = spaces.bot_reply(bot, delivery, conversation, text) {
                tracing::debug!(%why, "a space answer was not posted");
            }
        }
    }
    fn ended(&self, bot: &str, delivery: &str) {
        if let Some(spaces) = self.spaces.upgrade() {
            spaces.bot_ended(bot, delivery);
        }
    }
}

/// Connects the two services.
pub fn connect(potato: &Arc<Service>, spaces: &Arc<SpacesService>) {
    let link = Arc::new(Link { potato: Arc::downgrade(potato), spaces: Arc::downgrade(spaces) });
    potato.set_space_port(Some(link.clone()));
    spaces.set_bot_port(Some(link));
}

/// Connects whatever is running now: the Potato service and the Spaces service, when both exist.
pub fn connect_live() {
    if let (Some(potato), Some(spaces)) = (super::live::service(), crate::spaces::runtime::running_service()) {
        connect(&potato, &spaces);
    }
}

/// Cuts the Potato side loose (Spaces went off). The Spaces side drops its port when its service stops.
pub fn disconnect() {
    if let Some(potato) = super::live::service() {
        potato.set_space_port(None);
    }
}
