//! Desktop's `clientConfigStore.features`, with the free uses of its `freebieStore`.
use crate::bridge_login::Flow;
use crate::config::{millis_from_rfc3339, now_unix_secs};
use crate::models::NetworkSpec;
use serde_json::Value;

pub const FLAGS: &str = "com.beeper.feature_flags";
pub const USAGE: &str = "com.beeper.freebie_usage";
/// Desktop's `BRIDGE_ACCOUNT_SUMMARY`: the accounts that count against the plan's limits, and the limits.
pub const SUMMARY: &str = "com.beeper.bridge_account_summary";

/// Desktop's default `BRIDGE_ACCOUNT_SUMMARY` (`Bi`).
static DEFAULT_SUMMARY: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    serde_json::json!({
        "current": { "num_accounts": 0, "num_accounts_against_limit": 0, "bridges": {} },
        "max": { "num_accounts_against_limit": 10, "bridges": {} },
    })
});

/// Desktop's `platformBridgeKey`, the `network` of a platform, which keys the account limits.
fn network_key(n: &NetworkSpec) -> &'static str {
    match n.bridge_id {
        "tumblr" => "tumblrdms",
        id => id,
    }
}

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

/// The account's feature flags, free uses and account limits, and whether `whoami` reports a Plus subscription and
/// the internal channel.
#[derive(Debug, Clone, Default)]
pub struct Flags {
    pub flags: Value,
    pub usage: Value,
    /// Desktop's `bridgeAccountSummary`; `Null` until the account data comes.
    pub summary: Value,
    pub subscribed: bool,
    pub internal: bool,
}

impl Flags {
    /// The flags of an account on Desktop's free plan.
    pub fn free_plan() -> Self {
        let flags = serde_json::json!({
            "features": { "*": {
                "undo-send": false, "stealth-mode": false, "remind-later": false, "scheduled-message": true,
                "voice-transcription": false, "ai-in-beeper-chats": false, "enable-polls": true,
                "labels-v2": false, "android-inbox-rewrite": true, "enable-disappearing-message-timer-config": true,
                "enable-threads": true,
                "personal-room-overrides": true,
                "delete-chats": true,
            } },
            "freebies": [
                { "name": "voice-transcription", "type": "count", "default_limit": 5 },
                { "name": "scheduled-message", "type": "count", "default_limit": 5 },
                { "name": "remind-later", "type": "count", "default_limit": 5 },
                { "name": "incognito", "type": "duration", "duration_days": 3 },
                { "name": "labels", "type": "count", "default_limit": 1 },
            ],
        });
        let mut flags = flags;
        // The networks that the account can add, each on its Beeper Cloud bridge.
        let bridges = crate::config::NETWORK_SPECS.iter().filter_map(crate::config::cloud_bridge);
        bridges.for_each(|b| flags["features"]["*"][format!("bridge:{b}")] = true.into());
        // The limits of the free plan: ten accounts, one on each network.
        let networks = crate::config::NETWORK_SPECS[1..].iter().map(|n| (network_key(n).to_string(), 1.into()));
        let max = serde_json::json!({ "num_accounts_against_limit": 10, "bridges": Value::Object(networks.collect()) });
        let summary = serde_json::json!({ "current": DEFAULT_SUMMARY["current"], "max": max });
        Flags { flags, usage: Value::Null, summary, subscribed: false, internal: false }
    }

    /// Desktop's `updateBridgeAccountSummary`: the account data, or its default without it.
    fn summary(&self) -> &Value {
        if self.summary.is_null() {
            &DEFAULT_SUMMARY
        } else {
            &self.summary
        }
    }

    /// Desktop's `hasUnlimitedAccounts`: the plan has no total limit.
    pub fn unlimited_accounts(&self) -> bool {
        self.summary()["max"]["num_accounts_against_limit"].as_i64() == Some(-1)
    }

    /// Desktop's `accountsRemaining` (`sC`): the accounts that the plan still allows, `usize::MAX` without a limit.
    pub fn accounts_remaining(&self) -> usize {
        let s = self.summary();
        match s["max"]["num_accounts_against_limit"].as_i64().unwrap_or(0) {
            -1 => usize::MAX,
            max => (max - s["current"]["num_accounts_against_limit"].as_i64().unwrap_or(0)).max(0) as usize,
        }
    }

    /// Desktop's `addablePlatforms` of a network: the flag `bridge:{bridge}` of its Beeper Cloud or on-device bridge
    /// is on. Before the flags come, every network is.
    pub fn network_on(&self, network: &NetworkSpec) -> bool {
        let local = format!("local-{}", network_key(network));
        let mut bridges = crate::config::cloud_bridge(network).into_iter().chain([local.as_str()]);
        self.flags.is_null() || bridges.any(|b| self.flag(&format!("bridge:{b}")).on)
    }

    /// Desktop's `isPlanCapped` (`iC`) of a network: its accounts reached the plan's limit for it. Without a limit
    /// for it, a paid plan has none and the free plan allows none.
    pub fn plan_capped(&self, network: &NetworkSpec) -> bool {
        let (s, key) = (self.summary(), network_key(network));
        let max = s["max"]["bridges"][key].as_i64().unwrap_or(if self.subscribed { -1 } else { 0 });
        max != -1 && s["current"]["bridges"][key].as_i64().unwrap_or(0) >= max
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

    /// Desktop's `maxMergeChats`: the `max-merge-chats` flag, a whole number from -1 for no limit, else 1.
    pub fn max_merge_chats(&self) -> i64 {
        let all = &self.flags["features"];
        let v = all["appType:desktop"].get("max-merge-chats").or_else(|| all["*"].get("max-merge-chats"));
        let n = match v {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().parse().ok(),
            _ => None,
        };
        n.filter(|n| n.fract() == 0.0 && *n >= -1.0).map_or(1, |n| n as i64)
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

    /// Desktop's `freebieStore.stateFor` of the count freebie of the feature `key`, when the plan does not include the
    /// feature: the free uses that are used, and the limit.
    pub fn free_uses(&self, key: &str) -> Option<(i64, i64)> {
        let (_, left) = self.free(key).filter(|_| !self.flag(key).on)?;
        let used = self.usage[key]["used"].as_i64().unwrap_or(0);
        Some((used, used + left))
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

    /// Desktop's `supportsMultipleAccounts` (`WT`): the flag `bridge:{bridge}:multi-account`, else only the
    /// on-device and self-hosted bridges take more than one account.
    pub fn multi_account(&self, bridge: &str) -> bool {
        let flag = self.flag(&format!("bridge:{bridge}:multi-account"));
        match flag.show {
            true => flag.on,
            false => bridge.starts_with("local-") || bridge.starts_with("sh-"),
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
            "undo-send"
            | "ai-in-beeper-chats"
            | "enable-polls"
            | "enable-disappearing-message-timer-config"
            | "personal-room-overrides"
            | "drafts-sync"
            | "deferred-recovery-code-setup" => self.flag(key),
            "archive-as-unread" | "enable-threads" | "block-users" | "delete-chats" => {
                let on = self.flag(key).on;
                Feature { show: on, on, upgrade: false }
            }
            "labels-v2" => {
                let (on, inbox) = (self.flag(key).on, self.flag("android-inbox-rewrite").on);
                let free = !on && self.free("labels").is_some();
                Feature { show: inbox || on || free, on: on || free, upgrade: inbox && !on }
            }
            // Desktop's `features[SNOOZE]`: only for the internal channel, and open while a free snooze is.
            "snooze" if !self.internal => Feature { show: false, on: false, upgrade: false },
            "snooze" => match (self.flag(key), self.free(key)) {
                (f, Some(_)) if !f.on => Feature { show: true, on: true, ..f },
                (f, _) => f,
            },
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
        let mut f = Flags { flags, usage, summary: Value::Null, subscribed: false, internal: false };
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
        assert_eq!(f.free_uses("voice-transcription"), None, "the plan includes the feature");
        let mut free = Flags::free_plan();
        assert_eq!(free.free_uses("voice-transcription"), Some((0, 5)));
        free.usage = json!({ "voice-transcription": { "used": 5 } });
        assert_eq!(free.free_uses("voice-transcription"), Some((5, 5)));
    }

    /// Desktop's `features[SNOOZE]`: hidden outside the internal channel, else the flag, opened by a snooze freebie.
    #[test]
    fn snooze_is_for_the_internal_channel_like_desktop() {
        let mut f = Flags::free_plan();
        f.flags["features"]["*"]["snooze"] = json!(false);
        assert!(!f.get("snooze", 0).show, "only the internal channel snoozes");
        f.internal = true;
        assert_eq!(f.get("snooze", 0), Feature { show: true, on: false, upgrade: true });
        let snooze = json!({ "name": "snooze", "type": "count", "default_limit": 3 });
        f.flags["freebies"].as_array_mut().expect("the freebies").push(snooze);
        assert_eq!(f.get("snooze", 0), Feature { show: true, on: true, upgrade: true }, "a freebie opens it");
        f.usage = json!({ "snooze": { "used": 3 } });
        assert_eq!(f.free_uses("snooze"), Some((3, 3)), "the free snoozes are used up");
    }

    /// Desktop's `addAccountStore` limits from `com.beeper.bridge_account_summary`, in the shape of a real free account.
    #[test]
    fn account_limits_follow_the_summary_like_desktop() {
        let network = |id| crate::config::find_network(id).expect("a known network");
        let mut f = Flags::default();
        assert_eq!(f.accounts_remaining(), 10, "Desktop's default summary allows ten accounts");
        assert!(f.plan_capped(network("signal")), "the default free plan has no network limits");
        f.subscribed = true;
        assert!(!f.plan_capped(network("signal")), "a paid plan without a network limit has none");
        f.subscribed = false;
        f.summary = json!({
            "current": { "num_accounts": 6, "num_accounts_against_limit": 6, "bridges": { "discord": 1, "tumblrdms": 1 } },
            "max": { "num_accounts_against_limit": 12, "bridges": { "discord": 1, "signal": 1, "tumblrdms": 2 } },
            "fallback_plan": null,
        });
        assert_eq!((f.accounts_remaining(), f.unlimited_accounts()), (6, false));
        assert!(f.plan_capped(network("discord")) && !f.plan_capped(network("signal")));
        assert!(!f.plan_capped(network("tumblr")), "Tumblr's network is tumblrdms");
        f.summary["current"]["num_accounts_against_limit"] = json!(13);
        assert_eq!(f.accounts_remaining(), 0, "more accounts than the limit leave none");
        f.summary["max"]["num_accounts_against_limit"] = json!(-1);
        assert_eq!((f.accounts_remaining(), f.unlimited_accounts()), (usize::MAX, true));
        assert_eq!(Flags::free_plan().accounts_remaining(), 10);
        assert!(!Flags::free_plan().plan_capped(network("whatsapp")), "the free plan allows one account a network");
    }

    #[test]
    fn bridge_flags_turn_networks_on_like_desktop() {
        let network = |id| crate::config::find_network(id).expect("a known network");
        assert!(Flags::default().network_on(network("groupme")), "every network before the flags come");
        let f = Flags {
            flags: json!({ "features": { "*": {
                "bridge:groupme": true, "bridge:reddit": false, "bridge:local-tumblrdms": true, "bridge:discordgo": true,
            } } }),
            ..Flags::default()
        };
        assert!(f.network_on(network("groupme")) && !f.network_on(network("reddit")));
        assert!(f.network_on(network("discord")), "on its Beeper Cloud bridge discordgo");
        assert!(f.network_on(network("tumblr")), "on its on-device bridge local-tumblrdms");
        assert!(!f.network_on(network("muse")), "without a flag");
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
        assert!(!f.multi_account("whatsapp") && f.multi_account("local-whatsapp"));
        f.flags["features"]["*"]["bridge:whatsapp:multi-account"] = json!(true);
        assert!(f.multi_account("whatsapp"));
    }
}
