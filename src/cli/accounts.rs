//! The `accounts`, `contacts`, and `bridges` topics.

use super::chats::{account_ids, resolve_accounts, LIST_FLAGS};
use super::commands::cmd;
use super::render::Empty;
use super::{arg, config, config_dir, js_string, opt_arg, write_json, CliError, Cmd, Ctx, Flag, Res, BIN, NOT_FOUND};
use beeper_api::http::query_string;
use beeper_core::codec::url_encode;
use serde_json::{json, Value};

const SELECTOR: &str = "Account selector (ID, network, bridge, or user identity)";
const CONTACT_ACCOUNT: Flag = Flag::many("account", "Limit to Account ID, network, bridge, or account user");

pub const COMMANDS: &[Cmd] = &[
    Cmd {
        description: "`accounts add` without an argument opens the guided bridge chooser. Pass a bridge ID when you already know which chat network connector to use.",
        args: &[opt_arg("bridge", "Bridge ID, network, or type to connect. Omit to list available bridges.")],
        flags: &[
            Flag::many(
                "cookie",
                "Cookie value for non-interactive login, in name=value form. Repeat for multiple cookies.",
            ),
            Flag::many("field", "Field value for non-interactive login, in id=value form. Repeat for multiple fields."),
            Flag::s("flow", "Login flow ID. If omitted, Desktop chooses the default flow."),
            Flag::no("guided", "Prompt through login steps until completion").def("true"),
            Flag::s("login-id", "Existing login ID to re-login as"),
            Flag::b("non-interactive", "Do not prompt; require --flow, --field, and --cookie values when needed."),
            Flag::b("webview", "Use Bun.WebView to collect cookie login fields when a cookie step is returned."),
            Flag::s("webview-backend", "Bun.WebView backend for cookie login steps.")
                .opts(&["auto", "chrome", "webkit"])
                .def("chrome"),
            Flag::i("webview-timeout", "Seconds to wait for Bun.WebView cookie collection.").def("120"),
        ],
        ..cmd("accounts add", "Connect a chat account by bridge", super::account_login::accounts_add)
    },
    Cmd { flags: LIST_FLAGS, ..cmd("accounts chats", "List chats", super::chats::list) },
    Cmd {
        flags: &[Flag::many("account", "Filter by account selector"), Flag::b("ids", "Print only account IDs")],
        ..cmd("accounts list", "List connected accounts", accounts_list)
    },
    Cmd { args: &[arg("account", SELECTOR)], ..cmd("accounts remove", "Remove an account", accounts_remove) },
    Cmd { args: &[arg("account", SELECTOR)], ..cmd("accounts show", "Show account details", accounts_show) },
    Cmd {
        description: "Persists the choice in CLI config. Account-scoped commands that take --account fall back to this default when --account is omitted. Use `beeper accounts use \"\"` (or `beeper config set defaultAccount \"\"`) to clear.",
        args: &[arg("account", "Account selector (ID, network, bridge, user identity), or \"\" to clear.")],
        ..cmd("accounts use", "Select a default account for account-scoped commands", accounts_use)
    },
    Cmd {
        description: "`bridges list` is the scriptable bridge catalog. Use `accounts add` without an argument for the guided account connection flow.",
        flags: &[
            Flag::s("provider", "Limit to bridge provider").opts(&["local", "cloud", "self-hosted"]),
            Flag::no("available", "Only bridges available to add (--no-available to exclude)"),
        ],
        ..cmd("bridges list", "List bridges that can connect chat accounts", bridges_list)
    },
    Cmd {
        args: &[arg("bridge", "Bridge ID, display name, network, or type")],
        ..cmd("bridges show", "Show bridge details, login flows, and connected accounts", bridges_show)
    },
    Cmd {
        description: "List merged contacts for a specific account with cursor-based pagination.",
        flags: &[
            Flag::b("ids", "Print only contact user IDs"),
            Flag::i("limit", "Maximum contacts to print").def("50"),
            CONTACT_ACCOUNT,
            Flag::s("query", "Optional blended contact lookup query"),
        ],
        ..cmd("contacts list", "List contacts", contacts_list)
    },
    Cmd {
        description: "Search contacts on a specific account using merged account contacts, network search, and exact identifier lookup.",
        args: &[arg("query", "Contact search query")],
        flags: &[Flag::many("account", "Account ID, network, bridge, or account user. Omit to search every account.")],
        ..cmd("contacts search", "Search contacts", contacts_search)
    },
    Cmd {
        args: &[arg("id", "Contact user ID, display name, or phone/handle")],
        flags: &[Flag::many("account", "Limit to account ID, network, bridge, or account user")],
        ..cmd("contacts show", "Show contact details", contacts_show)
    },
];

fn accounts(c: &mut Ctx) -> Res<Vec<Value>> {
    Ok(match c.get("/v1/accounts")? {
        Value::Array(items) => items,
        mut page => page["items"].as_array_mut().map(std::mem::take).unwrap_or_default(),
    })
}

fn account_id(a: &Value) -> String {
    js_string(if a["accountID"].is_null() { &a["id"] } else { &a["accountID"] })
}

/// `resolveAccountID`: the one account of a selector.
fn resolve_one(c: &mut Ctx, input: &str) -> Res<String> {
    let ids = resolve_accounts(c, &[input.to_string()], false, true)?;
    ids.into_iter().next().ok_or_else(|| CliError::abort(format!("No account matches \"{input}\""), NOT_FOUND))
}

fn accounts_list(c: &mut Ctx) -> Res {
    let inputs = c.many("account").to_vec();
    let selected = resolve_accounts(c, &inputs, true, false)?;
    let default = config()?.get("defaultAccount").cloned().unwrap_or_default();
    let rows = accounts(c)?.into_iter().filter(|a| selected.is_empty() || selected.contains(&account_id(a)));
    let items: Vec<Value> = rows
        .map(|mut a| {
            if json!(account_id(&a)) == default {
                a["default"] = json!(true);
            }
            a
        })
        .collect();
    if c.on("ids") {
        items.iter().for_each(|a| println!("{}", account_id(a)));
        return Ok(());
    }
    c.print_list(&items, Empty::new("No accounts connected", format!("Run {BIN} accounts add to add one.")))
}

fn accounts_show(c: &mut Ctx) -> Res {
    let id = resolve_one(c, c.arg(0).expect("a required arg").to_string().as_str())?;
    let account = c.get(&format!("/v1/accounts/{}", url_encode(&id)))?;
    c.print(&account)
}

fn accounts_use(c: &mut Ctx) -> Res {
    c.writable()?;
    let input = c.arg(0).expect("a required arg").to_string();
    let mut config = config()?;
    if input.is_empty() {
        config.shift_remove("defaultAccount");
        write_json(&config_dir().join("config.json"), &config)?;
        return c.success("Cleared default account", None, json!({}));
    }
    let id = resolve_one(c, &input)?;
    config.insert("defaultAccount".into(), json!(id));
    write_json(&config_dir().join("config.json"), &config)?;
    c.success(&format!("Default account: {id}"), None, json!({ "accountID": id }))
}

/// Upstream asks the SDK for an account removal route, which the Desktop API does not have.
fn accounts_remove(c: &mut Ctx) -> Res {
    c.writable()?;
    resolve_one(c, c.arg(0).expect("a required arg").to_string().as_str())?;
    Err(CliError::bug("This Desktop API does not expose account removal."))
}

/// The account IDs of `--account`, or of every account.
fn contact_accounts(c: &mut Ctx) -> Res<Vec<String>> {
    let inputs = c.many("account").to_vec();
    match resolve_accounts(c, &inputs, true, true)? {
        ids if ids.is_empty() => account_ids(c),
        ids => Ok(ids),
    }
}

fn plural(n: usize, word: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { word } else { many })
}

fn contacts_list(c: &mut Ctx) -> Res {
    let ids = contact_accounts(c)?;
    let query: Vec<(&str, String)> = c.str("query").map(|q| ("query", q.to_string())).into_iter().collect();
    let limit = usize::try_from(c.int("limit").expect("--limit has a default")).unwrap_or(0);
    let load = |c: &mut Ctx| {
        let mut all: Vec<Value> = Vec::new();
        for id in &ids {
            if all.len() >= limit {
                break;
            }
            let path = format!("/v1/accounts/{}/contacts/list", url_encode(id));
            let page = c.collect(&path, &query, limit - all.len(), |_| true)?;
            all.extend(page.into_iter().map(|mut u| (u["accountID"] = json!(id), u).1));
        }
        Ok(all)
    };
    let items = match c.json() || c.on("ids") {
        true => load(c)?,
        false => {
            let label = c.str("query").map_or(String::new(), |q| format!(" matching \"{q}\""));
            c.spinner(&format!("Loading contacts{label}…"), load, |v| plural(v.len(), "contact", "contacts"))?
        }
    };
    if c.on("ids") {
        let ids: Vec<Value> = items
            .iter()
            .map(|u| json!({ "id": if u["userID"].is_null() { &u["id"] } else { &u["userID"] } }))
            .collect();
        return c.print_ids(&ids);
    }
    let subtitle =
        c.str("query").map_or("This account has no contacts to list.".into(), |q| format!("Nothing matched \"{q}\"."));
    let empty = Empty::new("No contacts found", subtitle)
        .try_("contacts search <query>", "narrow with a search")
        .try_("accounts", "check the account is online");
    c.print_list(&items, empty)
}

fn contacts_search(c: &mut Ctx) -> Res {
    let ids = contact_accounts(c)?;
    let q = c.arg(0).expect("a required arg").to_string();
    let load = |c: &mut Ctx| {
        let mut all: Vec<Value> = Vec::new();
        for id in &ids {
            // Some networks reject exact lookups for some identifiers, so the search goes on with the other accounts.
            if let Ok(mut found) =
                c.get(&query_string(&format!("/v1/accounts/{}/contacts", url_encode(id)), &[("query", &q)]))
            {
                let items = found["items"].as_array_mut().map(std::mem::take).unwrap_or_default();
                all.extend(items.into_iter().map(|mut u| (u["accountID"] = json!(id), u).1));
            }
        }
        Ok(all)
    };
    let accounts = plural(ids.len(), "account", "accounts");
    let results = match c.json() {
        true => load(c)?,
        false => {
            let done = |v: &Vec<Value>| format!("{} across {accounts}", plural(v.len(), "match", "matches"));
            c.spinner(&format!("Searching contacts for \"{q}\"…"), load, done)?
        }
    };
    let empty = Empty::new("No contacts matched", format!("Nothing across your {accounts} matched \"{q}\"."))
        .try_("accounts", "verify which accounts are connected")
        .try_(format!("contact <accountID> {q}"), "exact lookup on one account");
    c.print_list(&results, empty)
}

fn contacts_show(c: &mut Ctx) -> Res {
    let wanted = c.arg(0).expect("a required arg").to_string();
    let inputs = c.many("account").to_vec();
    for id in resolve_accounts(c, &inputs, true, true)? {
        let path = format!("/v1/accounts/{}/contacts/list", url_encode(&id));
        let page = c.collect(&path, &[("query", wanted.clone())], 10, |_| true)?;
        if let Some(contact) =
            page.into_iter().find(|u| ["userID", "id", "name", "displayName"].iter().any(|k| u[*k] == wanted))
        {
            return c.print(&json!({ "accountID": id, "contact": contact }));
        }
    }
    Err(CliError::abort(format!("Contact not found: {wanted}"), NOT_FOUND))
}

pub fn bridges(c: &mut Ctx) -> Res<Vec<Value>> {
    let mut list = c.get("/v1/bridges")?;
    Ok(list["items"].as_array_mut().map(std::mem::take).unwrap_or_default())
}

fn bridges_list(c: &mut Ctx) -> Res {
    let (provider, available) = (c.str("provider").map(str::to_string), c.opt_bool("available"));
    let items: Vec<Value> = bridges(c)?
        .into_iter()
        .filter(|b| provider.as_ref().is_none_or(|p| b["provider"] == p.as_str()))
        .filter(|b| available.is_none_or(|a| (b["status"] == "available") == a))
        .collect();
    let empty = Empty::new("No bridges matched", "Try removing provider or availability filters.")
        .try_("accounts add", "choose a bridge to connect an account");
    c.print_list(&items, empty)
}

/// `normalize` of `bridges/show.js`: lower case letters and digits.
fn letters(v: &Value) -> String {
    let s = if v.is_null() { String::new() } else { js_string(v) };
    s.to_lowercase().chars().filter(char::is_ascii_alphanumeric).collect()
}

/// The bridges whose ID, name, network, or type is `input`, else the bridges where one has it.
pub fn matching_bridges(items: Vec<Value>, input: &str) -> Vec<Value> {
    let wanted = letters(&json!(input));
    let keys = ["id", "displayName", "network", "type"];
    let (exact, rest): (Vec<Value>, Vec<Value>) =
        items.into_iter().partition(|b| keys.iter().any(|k| letters(&b[*k]) == wanted));
    let partial = |b: &Value| keys.iter().any(|k| letters(&b[*k]).contains(&wanted));
    if exact.is_empty() {
        rest.into_iter().filter(partial).collect()
    } else {
        exact
    }
}

/// `Name (id)` of each bridge.
pub fn bridge_names(all: &[Value]) -> String {
    let name = |b: &Value| js_string(if b["displayName"].is_null() { &b["id"] } else { &b["displayName"] });
    all.iter().map(|b| format!("{} ({})", name(b), js_string(&b["id"]))).collect::<Vec<_>>().join(", ")
}

/// `resolveBridge`: an exact ID, name, network, or type, else a part of one.
fn resolve_bridge(items: Vec<Value>, input: &str) -> Res<Value> {
    let mut matches = matching_bridges(items, input);
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(CliError::bug(format!("Unknown bridge \"{input}\". Run `{BIN} bridges list`."))),
        _ => Err(CliError::bug(format!("Bridge \"{input}\" is ambiguous. Use one of: {}", bridge_names(&matches)))),
    }
}

fn bridges_show(c: &mut Ctx) -> Res {
    let input = c.arg(0).expect("a required arg").to_string();
    let listed = resolve_bridge(bridges(c)?, &input)?;
    let path = format!("/v1/bridges/{}", url_encode(&js_string(&listed["id"])));
    let mut bridge = c.get(&path).unwrap_or(listed);
    let flows =
        c.get(&format!("{path}/login-flows")).ok().map(|f| if f["items"].is_null() { f } else { f["items"].clone() });
    let capabilities = c.get(&format!("{path}/capabilities")).ok();
    let object = bridge.as_object_mut().expect("a bridge is an object");
    for (key, value) in [("loginFlows", flows), ("capabilities", capabilities)] {
        match value {
            Some(v) => drop(object.insert(key.into(), v)),
            None => drop(object.shift_remove(key)),
        }
    }
    c.print(&bridge)
}
