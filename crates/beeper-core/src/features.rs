//! Desktop's `clientConfigStore.features`, with the free uses of its `freebieStore`.
use crate::config::{millis_from_rfc3339, now_unix_secs};
use serde_json::Value;

pub const FLAGS: &str = "com.beeper.feature_flags";
pub const USAGE: &str = "com.beeper.freebie_usage";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Feature {
    pub show: bool,
    pub on: bool,
    /// Desktop's `requiresUpgrade`: the feature is part of a paid plan, with a Plus pill.
    pub upgrade: bool,
}

impl Feature {
    /// Desktop's `ui`: the controls of the feature are off and open the paywall.
    pub fn locked(self) -> bool {
        self.upgrade && !self.on
    }
}

/// The account's feature flags and free uses, and whether `whoami` reports a Plus subscription.
#[derive(Debug, Clone, Default)]
pub struct Flags {
    pub flags: Value,
    pub usage: Value,
    pub subscribed: bool,
}

impl Flags {
    /// The flags of an account on Desktop's free plan.
    pub fn free_plan() -> Self {
        let flags = serde_json::json!({
            "features": { "*": {
                "undo-send": false, "stealth-mode": false, "remind-later": false, "scheduled-message": true,
                "voice-transcription": false, "ai-in-beeper-chats": false, "enable-polls": true,
                "labels-v2": false, "android-inbox-rewrite": true,
            } },
            "freebies": [
                { "name": "voice-transcription", "type": "count", "default_limit": 5 },
                { "name": "scheduled-message", "type": "count", "default_limit": 5 },
                { "name": "remind-later", "type": "count", "default_limit": 5 },
                { "name": "incognito", "type": "duration", "duration_days": 3 },
                { "name": "labels", "type": "count", "default_limit": 1 },
            ],
        });
        Flags { flags, usage: Value::Null, subscribed: false }
    }

    /// Desktop's `combinedFeatures`: the `appType:desktop` flags override the `*` flags.
    fn flag(&self, key: &str) -> Feature {
        let all = &self.flags["features"];
        let v = all["appType:desktop"].get(key).or_else(|| all["*"].get(key));
        v.and_then(Value::as_bool).map_or(Feature { show: false, on: false, upgrade: true }, |on| Feature {
            show: true,
            on,
            upgrade: true,
        })
    }

    /// Desktop's `freebieStore.stateFor`: whether a free use or a trial is open, and the free uses left.
    fn free(&self, name: &str) -> Option<(bool, i64)> {
        let def = self.flags["freebies"].as_array()?.iter().find(|d| d["name"] == name)?;
        if self.subscribed {
            return None;
        }
        let used = &self.usage[name];
        if def["type"] == "duration" {
            let ends = used["expires_at"].as_str().map(|t| millis_from_rfc3339(t).expect("an RFC 3339 trial end"));
            return Some((ends.is_some_and(|t| now_unix_secs() * 1000 < t), i64::from(ends.is_none())));
        }
        let limit = used["limit"].as_i64().or(def["default_limit"].as_i64()).expect("a freebie limit");
        let left = (limit - used["used"].as_i64().unwrap_or(0)).max(0);
        Some((left > 0, left))
    }

    /// Desktop's `trialDaysLeft` of the incognito trial: the days left, rounded up, while the trial is open.
    pub fn trial_days_left(&self) -> Option<i64> {
        const DAY: i64 = 86_400_000;
        let (open, _) = self.free("incognito")?;
        let ends = self.usage["incognito"]["expires_at"].as_str()?;
        let left = millis_from_rfc3339(ends).expect("an RFC 3339 trial end") - now_unix_secs() * 1000;
        open.then(|| ((left + DAY - 1) / DAY).max(1))
    }

    /// Desktop's `features[key]`; `scheduled` is the number of pending scheduled messages.
    pub fn get(&self, key: &str, scheduled: usize) -> Feature {
        let with_free = |f: Feature, name: &str| match self.free(name) {
            Some((open, _)) if !f.on => Feature { show: true, on: open, ..f },
            _ => f,
        };
        match key {
            "stealth-mode" => with_free(self.flag(key), "incognito"),
            "remind-later" | "voice-transcription" => with_free(self.flag(key), key),
            "talk-to-type" => self.flag("voice-transcription"),
            "undo-send" | "ai-in-beeper-chats" | "enable-polls" => self.flag(key),
            "labels-v2" => {
                let (on, inbox) = (self.flag(key).on, self.flag("android-inbox-rewrite").on);
                let free = !on && self.free("labels").is_some();
                Feature { show: inbox || on || free, on: on || free, upgrade: inbox && !on }
            }
            "scheduled-message" => match (self.flag(key), self.free(key)) {
                (f, Some((open, left))) if !f.on => Feature { show: true, on: open && (scheduled as i64) < left, ..f },
                (f, _) => f,
            },
            _ => panic!("unknown feature {key}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn features_follow_desktop() {
        let flags = json!({
            "features": { "*": { "undo-send": false, "stealth-mode": false, "remind-later": false, "scheduled-message": false, "voice-transcription": true } },
            "freebies": [
                { "name": "incognito", "type": "duration", "duration_days": 3 },
                { "name": "remind-later", "type": "count", "default_limit": 5 },
                { "name": "scheduled-message", "type": "count", "default_limit": 2 },
            ],
        });
        let usage = json!({ "incognito": { "started_at": "2020-01-01T00:00:00Z", "expires_at": "2020-01-04T00:00:00Z" }, "remind-later": { "used": 5 } });
        let mut f = Flags { flags, usage, subscribed: false };
        let lock = Feature { show: true, on: false, upgrade: true };
        let open = Feature { on: true, ..lock };
        assert_eq!(f.get("undo-send", 0), lock);
        assert_eq!(f.get("stealth-mode", 0), lock, "the trial ended");
        assert_eq!(f.get("remind-later", 0), lock, "no free uses are left");
        assert_eq!(f.get("scheduled-message", 1), open);
        assert_eq!(f.get("scheduled-message", 2), lock, "each pending message takes a free use");
        assert_eq!(f.get("talk-to-type", 0), open);
        assert!(!f.get("ai-in-beeper-chats", 0).show, "a missing flag hides the feature");
        assert!(!f.get("enable-polls", 0).on, "a missing flag hides polls");
        assert!(Flags::free_plan().get("enable-polls", 0).on);
        assert!(!f.get("labels-v2", 0).show, "no flag and no freebie hides labels");
        assert_eq!(Flags::free_plan().get("labels-v2", 0), Feature { show: true, on: true, upgrade: true });
        f.subscribed = true;
        assert_eq!(f.get("scheduled-message", 0), lock, "subscribers get no free uses");
        f.usage =
            json!({ "incognito": { "started_at": "2020-01-01T00:00:00Z", "expires_at": "2999-01-01T00:00:00Z" } });
        f.subscribed = false;
        assert_eq!(f.get("stealth-mode", 0), open, "the trial is open");
    }
}
