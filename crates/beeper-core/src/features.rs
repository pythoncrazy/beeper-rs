//! Desktop's `clientConfigStore.features`, with the free uses of its `freebieStore`.
use crate::bridge_login::Flow;
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
                "labels-v2": false, "android-inbox-rewrite": true, "enable-disappearing-message-timer-config": true,
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

    /// Desktop's `freebieStore.deriveState`: whether a free use or a trial is open, and the free uses left.
    /// A bad trial end or a missing limit gives Desktop's `NaN`, which is not open.
    fn free(&self, name: &str) -> Option<(bool, i64)> {
        let def = self.flags["freebies"].as_array()?.iter().find(|d| d["name"] == name)?;
        if self.subscribed {
            return None;
        }
        let used = &self.usage[name];
        if def["type"] == "duration" {
            let started = used.get("started_at").is_some();
            let ends = used["expires_at"].as_str().and_then(millis_from_rfc3339).filter(|_| started);
            return Some((ends.is_some_and(|t| now_unix_secs() * 1000 < t), i64::from(!started)));
        }
        let used = used.get("used").map(|_| used);
        let Some(limit) = used.and_then(|u| u["limit"].as_i64()).or(def["default_limit"].as_i64()) else {
            return Some((false, 0));
        };
        let left = (limit - used.and_then(|u| u["used"].as_i64()).unwrap_or(0)).max(0);
        Some((left > 0, left))
    }

    /// Desktop's `trialDaysLeft` of the incognito trial: the days left, rounded up, while the trial is open.
    pub fn trial_days_left(&self) -> Option<i64> {
        const DAY: i64 = 86_400_000;
        let (open, _) = self.free("incognito")?;
        let ends = self.usage["incognito"]["expires_at"].as_str().and_then(millis_from_rfc3339)?;
        let left = ends - now_unix_secs() * 1000;
        open.then(|| ((left + DAY - 1) / DAY).max(1))
    }

    /// The SDK's `filterDisabledLoginFlows`: the flag `bridge:{bridge}:login:{flow}` set to false hides a flow,
    /// unless it hides all of them.
    pub fn login_flows(&self, bridge: &str, flows: Vec<Flow>) -> Vec<Flow> {
        let shown: Vec<Flow> = flows
            .iter()
            .filter(|f| {
                let flag = self.flag(&format!("bridge:{bridge}:login:{}", f.id));
                !flag.show || flag.on
            })
            .cloned()
            .collect();
        if shown.is_empty() {
            flows
        } else {
            shown
        }
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
            "undo-send" | "ai-in-beeper-chats" | "enable-polls" | "enable-disappearing-message-timer-config" => {
                self.flag(key)
            }
            "archive-as-unread" => {
                let on = self.flag(key).on;
                Feature { show: on, on, upgrade: false }
            }
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
        assert!(!f.get("archive-as-unread", 0).show, "a missing flag hides the archive-as-unread setting");
        f.flags["features"]["*"]["archive-as-unread"] = json!(true);
        assert_eq!(f.get("archive-as-unread", 0), Feature { show: true, on: true, upgrade: false });
        assert_eq!(Flags::free_plan().get("labels-v2", 0), Feature { show: true, on: true, upgrade: true });
        f.subscribed = true;
        assert_eq!(f.get("scheduled-message", 0), lock, "subscribers get no free uses");
        f.usage =
            json!({ "incognito": { "started_at": "2020-01-01T00:00:00Z", "expires_at": "2999-01-01T00:00:00Z" } });
        f.subscribed = false;
        assert_eq!(f.get("stealth-mode", 0), open, "the trial is open");
        f.usage = json!({ "incognito": { "started_at": "2020-01-01T00:00:00Z", "expires_at": "soon" } });
        assert_eq!((f.get("stealth-mode", 0), f.trial_days_left()), (lock, None), "a bad trial end is not open");
        f.usage = json!({ "incognito": { "expires_at": "2999-01-01T00:00:00Z" } });
        assert_eq!(f.get("stealth-mode", 0), lock, "a trial without a start did not start");
        f.flags["freebies"][1] = json!({ "name": "remind-later", "type": "count" });
        f.usage = json!({ "remind-later": { "used": 1 } });
        assert_eq!(f.get("remind-later", 0), lock, "a missing limit gives no free uses");
        f.usage = json!({ "scheduled-message": { "limit": 9 } });
        assert_eq!(f.get("scheduled-message", 2), lock, "a usage without a count keeps the default limit");
    }

    #[test]
    fn feature_flags_hide_login_flows_like_the_sdk() {
        let flow = |id: &str| Flow { id: id.into(), name: String::new(), description: String::new() };
        let ids = |flows: Vec<Flow>| flows.into_iter().map(|f| f.id).collect::<Vec<_>>();
        let mut f = Flags::free_plan();
        f.flags["features"]["appType:desktop"] = json!({ "bridge:telegram:login:qr": false });
        assert_eq!(ids(f.login_flows("telegram", vec![flow("phone"), flow("qr")])), ["phone"]);
        assert_eq!(ids(f.login_flows("signal", vec![flow("qr")])), ["qr"], "the flag is per bridge");
        assert_eq!(ids(f.login_flows("telegram", vec![flow("qr")])), ["qr"], "a flag does not hide every flow");
    }
}
