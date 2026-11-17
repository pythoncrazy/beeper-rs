//! Desktop's translations of the texts of the bridge login steps (`rs` and `Za` of `BridgeV2AuthComponent`): each
//! flow name, flow description, field label, field description, field placeholder, instruction and error has a key
//! and a gettext context, and its translation takes the place of the bridge's English text.

/// Desktop's `Zf`: the kind, the key and the context of each translated bridge text, from `scripts/i18n.py`'s Desktop.
#[rustfmt::skip]
const CONTEXTS: &[(&str, &str, &str)] = &[
    ("error", "Challenge required, please check the official website or app and then try again", "BridgeV2 Instagram/Facebook/Messenger login error: Challenge required, please check the official website or app and then try again"),
    ("error", "Checkpoint required, please check the official website or app and then try again", "BridgeV2 Instagram/Facebook/Messenger login error: Checkpoint required, please check the official website or app and then try again"),
    ("error", "Consent required, please check the official website or app and then try again", "BridgeV2 Instagram/Facebook/Messenger login error: Consent required, please check the official website or app and then try again"),
    ("error", "Couldn't read your X account ID. Please try again.", "BridgeV2 X/Twitter login error: Couldn't read your X account ID. Please try again."),
    ("error", "Couldn't verify your passcode. Please try again.", "BridgeV2 X/Twitter login error: Couldn't verify your passcode. Please try again."),
    ("error", "Entering code or scanning QR timed out. Please try again.", "BridgeV2 WhatsApp login error: Entering code or scanning QR timed out. Please try again."),
    ("error", "Got client outdated error while waiting for QRs. The bridge must be updated to continue.", "BridgeV2 WhatsApp login error"),
    ("error", "Got logged out immediately", "BridgeV2 Instagram/Facebook/Messenger login error: Got logged out immediately"),
    ("error", "Internal error logging in", "BridgeV2 Instagram/Facebook/Messenger login error: Internal error logging in"),
    ("error", "Invalid password", "BridgeV2 Telegram login error: Invalid password"),
    ("error", "Invalid phone code", "BridgeV2 Telegram login error: Invalid phone code"),
    ("error", "Missing cookies", "BridgeV2 Instagram/Facebook/Messenger login error: Missing cookies"),
    ("error", "New account creation is not supported", "BridgeV2 Telegram login error: New account creation is not supported"),
    ("error", "Passcode isn't set up for X Chat. Set it up on x.com.", "BridgeV2 X/Twitter login error: Passcode isn't set up for X Chat. Set it up on x.com."),
    ("error", "Phone number must be in international format", "BridgeV2 WhatsApp login error: Phone number must be in international format"),
    ("error", "Phone number too short", "BridgeV2 WhatsApp login error: Phone number too short"),
    ("error", "Please enable WhatsApp web multidevice and scan the QR code again.", "BridgeV2 WhatsApp login error: Please enable WhatsApp web multidevice and scan the QR code again."),
    ("error", "Rate limited by WhatsApp. Try again later.", "BridgeV2 WhatsApp login error: Rate limited by WhatsApp. Try again later."),
    ("error", "Temporary error. Try again.", "BridgeV2 X/Twitter login error: Temporary error. Try again."),
    ("error", "This bridge is out of date. Update and try again.", "BridgeV2 X/Twitter login error: This bridge is out of date. Update and try again."),
    ("error", "Too many attempts. Please try again later.", "BridgeV2 X/Twitter login error: Too many attempts. Please try again later."),
    ("error", "Too many incorrect passcode attempts. X Chat is locked.", "BridgeV2 X/Twitter login error: Too many incorrect passcode attempts. X Chat is locked."),
    ("error", "Unexpected event while waiting for login", "BridgeV2 WhatsApp login error: Unexpected event while waiting for login"),
    ("error", "Viber device linking is not configured for this bridge.", "BridgeV2 Viber login error: Viber device linking is not configured for this bridge."),
    ("fieldDescription", "com.beeper.ai.login.beeper/display_name", "BridgeV2 Beeper AI login field description: Name shown for this AI configuration"),
    ("fieldDescription", "com.beeper.ai.login.beeper/login_id", "BridgeV2 Beeper AI login field description: Stable ID for this AI configuration"),
    ("fieldDescription", "com.beeper.ai.login.provider.config/api_key", "BridgeV2 Beeper AI login field description: Provider API key"),
    ("fieldDescription", "com.beeper.ai.login.provider.config/base_url", "BridgeV2 Beeper AI login field description: OpenAI-compatible API base URL"),
    ("fieldDescription", "com.beeper.ai.login.provider.config/provider_id", "BridgeV2 Beeper AI login field description: Stable provider ID"),
    ("fieldDescription", "com.beeper.ai.login.provider.default_model/default_model", "BridgeV2 Beeper AI login field description: Model to use by default"),
    ("fieldDescription", "fi.mau.telegram.login.code/fi.mau.telegram.login.code", "BridgeV2 Telegram login field description: The code was sent to the Telegram app on your phone"),
    ("fieldDescription", "fi.mau.telegram.login.phone_number/fi.mau.telegram.login.phone_number", "BridgeV2 Telegram login field description: Include the country code with +"),
    ("fieldDescription", "fi.mau.viber.login.phone/phone_number", "BridgeV2 Viber login field description: Your Viber phone number in international format"),
    ("fieldDescription", "fi.mau.whatsapp.login.phone/phone_number", "BridgeV2 WhatsApp login field description: Your WhatsApp phone number in international format"),
    ("fieldLabel", "com.beeper.ai.login.beeper/display_name", "BridgeV2 Beeper AI login field label: Display name"),
    ("fieldLabel", "com.beeper.ai.login.beeper/login_id", "BridgeV2 Beeper AI login field label: Login ID"),
    ("fieldLabel", "com.beeper.ai.login.provider.config/api_key", "BridgeV2 Beeper AI login field label: API key"),
    ("fieldLabel", "com.beeper.ai.login.provider.config/base_url", "BridgeV2 Beeper AI login field label: Base URL"),
    ("fieldLabel", "com.beeper.ai.login.provider.config/provider_id", "BridgeV2 Beeper AI login field label: ID"),
    ("fieldLabel", "com.beeper.ai.login.provider.default_model/default_model", "BridgeV2 Beeper AI login field label: Default model"),
    ("fieldLabel", "fi.mau.discord.enter_token/token", "BridgeV2 Discord login field label: Discord user account token"),
    ("fieldLabel", "fi.mau.telegram.login.bot_token/fi.mau.telegram.login.bot_token", "BridgeV2 Telegram login field label: Bot token"),
    ("fieldLabel", "fi.mau.telegram.login.code.incorrect/fi.mau.telegram.login.code", "BridgeV2 Telegram login field label: Code"),
    ("fieldLabel", "fi.mau.telegram.login.code/fi.mau.telegram.login.code", "BridgeV2 Telegram login field label: Code"),
    ("fieldLabel", "fi.mau.telegram.login.manual_session/fi.mau.telegram.login.manual_session", "BridgeV2 Telegram login field label: Session data JSON"),
    ("fieldLabel", "fi.mau.telegram.login.password.incorrect/fi.mau.telegram.login.password", "BridgeV2 Telegram login field label: Password"),
    ("fieldLabel", "fi.mau.telegram.login.password/fi.mau.telegram.login.password", "BridgeV2 Telegram login field label: Password"),
    ("fieldLabel", "fi.mau.telegram.login.phone_number/fi.mau.telegram.login.phone_number", "BridgeV2 Telegram login field label: Phone number"),
    ("fieldLabel", "fi.mau.viber.login.phone/phone_number", "BridgeV2 Viber login field label: Phone number"),
    ("fieldLabel", "fi.mau.whatsapp.login.phone/phone_number", "BridgeV2 WhatsApp login field label: Phone number"),
    ("flowDescription", "ai/beeper", "BridgeV2 Beeper AI login flow description: Use the default Beeper AI provider"),
    ("flowDescription", "ai/chatgpt-device", "BridgeV2 Beeper AI login flow description: Log in with ChatGPT using a browser device code"),
    ("flowDescription", "ai/openai-codex-responses", "BridgeV2 Beeper AI login flow description: Add a provider using the OpenAI Codex Responses API"),
    ("flowDescription", "ai/openai-completions", "BridgeV2 Beeper AI login flow description: Add a provider using the OpenAI chat completions API"),
    ("flowDescription", "ai/openai-responses", "BridgeV2 Beeper AI login flow description: Add a provider using the OpenAI Responses API"),
    ("flowDescription", "discordgo/qr", "BridgeV2 Discord login flow description: Scan a QR code with the Discord mobile app to log in."),
    ("flowDescription", "discordgo/token", "BridgeV2 Discord login flow description: Log in to your Discord account in a web browser."),
    ("flowDescription", "facebookgo/facebook", "BridgeV2 Facebook/Messenger login flow description: Login using cookies from facebook.com"),
    ("flowDescription", "facebookgo/instagram", "BridgeV2 Facebook/Messenger login flow description: Login using cookies from instagram.com"),
    ("flowDescription", "facebookgo/messenger", "BridgeV2 Facebook/Messenger login flow description: Login using cookies from messenger.com"),
    ("flowDescription", "facebookgo/messenger-lite", "BridgeV2 Facebook/Messenger login flow description: Log in using Messenger mobile API"),
    ("flowDescription", "gmessages/google", "BridgeV2 Google Messages login flow description: Log in with your Google account and pair by tapping an emoji on your phone"),
    ("flowDescription", "gmessages/qr", "BridgeV2 Google Messages login flow description: Pair by scanning a QR code on your phone"),
    ("flowDescription", "gvoice/cookies", "BridgeV2 Google Voice login flow description: Log in with your Google account by providing cookies from voice.google.com"),
    ("flowDescription", "instagramgo/facebook", "BridgeV2 Instagram/Messenger login flow description: Login using cookies from facebook.com"),
    ("flowDescription", "instagramgo/instagram", "BridgeV2 Instagram/Messenger login flow description: Login using cookies from instagram.com"),
    ("flowDescription", "instagramgo/messenger", "BridgeV2 Instagram/Messenger login flow description: Login using cookies from messenger.com"),
    ("flowDescription", "instagramgo/messenger-lite", "BridgeV2 Instagram/Messenger login flow description: Log in using Messenger mobile API"),
    ("flowDescription", "linkedin/cookies", "BridgeV2 LinkedIn login flow description: Log in with your LinkedIn account using your cookies"),
    ("flowDescription", "signal/qr", "BridgeV2 Signal login flow description: Scan a QR code to pair the bridge to your Signal app"),
    ("flowDescription", "telegram/bot", "BridgeV2 Telegram login flow description: Log in as a bot using the bot token provided by BotFather"),
    ("flowDescription", "telegram/manual", "BridgeV2 Telegram login flow description: Log in using existing session credentials (advanced, do not use)"),
    ("flowDescription", "telegram/phone", "BridgeV2 Telegram login flow description: Login using your Telegram phone number"),
    ("flowDescription", "telegram/qr", "BridgeV2 Telegram login flow description: Login by scanning a QR code from your phone"),
    ("flowDescription", "twitter/cookies", "BridgeV2 X/Twitter login flow description: Log in with your X account using your cookies"),
    ("flowDescription", "viber/qr", "BridgeV2 Viber login flow description: Scan a QR code with Viber on your phone to link Beeper as a device"),
    ("flowDescription", "wechat/phone", "BridgeV2 WeChat login flow description: Approve the login on your phone, or scan a QR code if this device is not linked"),
    ("flowDescription", "wechat/qr", "BridgeV2 WeChat login flow description: Scan a QR code with WeChat on your phone to link Beeper as a device"),
    ("flowDescription", "whatsapp/phone", "BridgeV2 WhatsApp login flow description"),
    ("flowDescription", "whatsapp/qr", "BridgeV2 WhatsApp login flow description: Scan a QR code to pair the bridge to your WhatsApp account"),
    ("flowName", "ai/beeper", "BridgeV2 Beeper AI login flow name: Beeper AI"),
    ("flowName", "ai/chatgpt-device", "BridgeV2 Beeper AI login flow name: ChatGPT"),
    ("flowName", "ai/openai-codex-responses", "BridgeV2 Beeper AI login flow name: OpenAI Codex Responses"),
    ("flowName", "ai/openai-completions", "BridgeV2 Beeper AI login flow name: OpenAI Chat Completions"),
    ("flowName", "ai/openai-responses", "BridgeV2 Beeper AI login flow name: OpenAI Responses"),
    ("flowName", "discordgo/qr", "BridgeV2 Discord login flow name: QR Code"),
    ("flowName", "discordgo/token", "BridgeV2 Discord login flow name: Browser"),
    ("flowName", "facebookgo/facebook", "BridgeV2 Facebook/Messenger login flow name: facebook.com"),
    ("flowName", "facebookgo/instagram", "BridgeV2 Facebook/Messenger login flow name: instagram.com"),
    ("flowName", "facebookgo/messenger", "BridgeV2 Facebook/Messenger login flow name: messenger.com"),
    ("flowName", "facebookgo/messenger-lite", "BridgeV2 Facebook/Messenger login flow name: Messenger iOS"),
    ("flowName", "gmessages/google", "BridgeV2 Google Messages login flow name: Google Account"),
    ("flowName", "gmessages/qr", "BridgeV2 Google Messages login flow name: QR"),
    ("flowName", "gvoice/cookies", "BridgeV2 Google Voice login flow name: Cookies"),
    ("flowName", "instagramgo/facebook", "BridgeV2 Instagram/Messenger login flow name: facebook.com"),
    ("flowName", "instagramgo/instagram", "BridgeV2 Instagram/Messenger login flow name: instagram.com"),
    ("flowName", "instagramgo/messenger", "BridgeV2 Instagram/Messenger login flow name: messenger.com"),
    ("flowName", "instagramgo/messenger-lite", "BridgeV2 Instagram/Messenger login flow name: Messenger iOS"),
    ("flowName", "linkedin/cookies", "BridgeV2 LinkedIn login flow name: Cookies"),
    ("flowName", "signal/qr", "BridgeV2 Signal login flow name: QR"),
    ("flowName", "telegram/bot", "BridgeV2 Telegram login flow name: Bot token"),
    ("flowName", "telegram/manual", "BridgeV2 Telegram login flow name: Manual"),
    ("flowName", "telegram/phone", "BridgeV2 Telegram login flow name: Phone Number"),
    ("flowName", "telegram/qr", "BridgeV2 Telegram login flow name: QR Code"),
    ("flowName", "twitter/cookies", "BridgeV2 X/Twitter login flow name: Cookies"),
    ("flowName", "viber/qr", "BridgeV2 Viber login flow name: QR"),
    ("flowName", "wechat/phone", "BridgeV2 WeChat login flow name: Approve on phone"),
    ("flowName", "wechat/qr", "BridgeV2 WeChat login flow name: QR"),
    ("flowName", "whatsapp/phone", "BridgeV2 WhatsApp login flow name: Pairing code"),
    ("flowName", "whatsapp/qr", "BridgeV2 WhatsApp login flow name: QR"),
    ("instructions", "com.beeper.ai.login.beeper", "BridgeV2 Beeper AI login step instructions: Create an AI bridge login"),
    ("instructions", "com.beeper.ai.login.provider.config", "BridgeV2 Beeper AI login step instructions: Enter provider connection details"),
    ("instructions", "com.beeper.ai.login.provider.default_model", "BridgeV2 Beeper AI login step instructions: Choose the default model"),
    ("instructions", "fi.mau.WeChat.login.phone", "BridgeV2 WeChat login step instructions"),
    ("instructions", "fi.mau.WeChat.login.qr", "BridgeV2 WeChat login step instructions"),
    ("instructions", "fi.mau.discord.cookies", "BridgeV2 Discord login step instructions: Log in with Discord."),
    ("instructions", "fi.mau.discord.qr", "BridgeV2 Discord login step instructions: On your phone, find “Scan QR Code” in Discord’s settings."),
    ("instructions", "fi.mau.gmessages.emoji", "BridgeV2 Google Messages login step instructions: Tap the emoji on the Google Messages app on your phone"),
    ("instructions", "fi.mau.gmessages.google_account", "BridgeV2 Google Messages login step instructions"),
    ("instructions", "fi.mau.gmessages.qr", "BridgeV2 Google Messages login step instructions: Scan the QR code using the Google Messages app on your Android phone"),
    ("instructions", "fi.mau.gvoice.cookies", "BridgeV2 Google Voice login step instructions"),
    ("instructions", "fi.mau.linkedin.login.enter_cookies", "BridgeV2 LinkedIn login step instructions"),
    ("instructions", "fi.mau.meta.cookies", "BridgeV2 Instagram/Facebook/Messenger login step instructions"),
    ("instructions", "fi.mau.signal.login.qr", "BridgeV2 Signal login step instructions: Scan the QR code on your Signal app to log in"),
    ("instructions", "fi.mau.telegram.login.bot_token", "BridgeV2 Telegram login step instructions: Please enter the bot token you want to log in as"),
    ("instructions", "fi.mau.telegram.login.code.incorrect", "BridgeV2 Telegram login step instructions: Incorrect code"),
    ("instructions", "fi.mau.telegram.login.password", "BridgeV2 Telegram login step instructions: You have two-factor authentication enabled."),
    ("instructions", "fi.mau.telegram.login.password.incorrect", "BridgeV2 Telegram login step instructions"),
    ("instructions", "fi.mau.telegram.login.show_qr", "BridgeV2 Telegram login step instructions: Scan the QR code on your phone to log in"),
    ("instructions", "fi.mau.twitter.login.enter_cookies", "BridgeV2 X/Twitter login step instructions"),
    ("instructions", "fi.mau.viber.login.complete", "BridgeV2 Viber login step instructions"),
    ("instructions", "fi.mau.viber.login.qr", "BridgeV2 Viber login step instructions"),
    ("instructions", "fi.mau.whatsapp.login.code", "BridgeV2 WhatsApp login step instructions: Input the pairing code in the WhatsApp mobile app to log in"),
    ("instructions", "fi.mau.whatsapp.login.qr", "BridgeV2 WhatsApp login step instructions: Scan the QR code with the WhatsApp mobile app to log in"),
];

/// Desktop's `eg`: the context of a key of a kind that `Zf` does not list.
fn default_context(kind: &str) -> &'static str {
    match kind {
        "instructions" => "BridgeV2 login instructions",
        "flowName" => "BridgeV2 login flow name",
        "flowDescription" => "BridgeV2 login flow description",
        "fieldLabel" => "BridgeV2 login field label",
        "fieldDescription" => "BridgeV2 login field description",
        "fieldPlaceholder" => "BridgeV2 login field placeholder",
        "error" => "BridgeV2 login error",
        other => panic!("unknown bridge text kind {other}"),
    }
}

/// Desktop's `Qf`: the instructions whose key names different texts, which stay the bridge's.
const AMBIGUOUS: [&str; 2] = ["com.beeper.ai.login.complete", "fi.mau.gmessages.complete"];

/// Desktop's `rs`: the translation of the bridge's `text` of `kind` with `key` (a step ID, `step/field` or
/// `bridge/flow`), or the text itself when it has no translation.
pub(crate) fn bridge_text(kind: &str, key: &str, text: &str) -> String {
    bridge_text_in(None, kind, key, text)
}

fn bridge_text_in(code: Option<&str>, kind: &str, key: &str, text: &str) -> String {
    if text.is_empty() || key.is_empty() || (kind == "instructions" && AMBIGUOUS.contains(&key)) {
        return text.into();
    }
    let context = CONTEXTS.iter().find(|c| c.0 == kind && c.1 == key).map_or_else(|| default_context(kind), |c| c.2);
    crate::i18n::translate_in(code, key, context).filter(|t| t != key).unwrap_or_else(|| text.into())
}

/// Desktop's `Za`: the translation of the bridge's error, whose key is its text.
pub(crate) fn bridge_error(text: &str) -> String {
    bridge_text("error", text, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Desktop's `rs` in German: the translated flow name and error, and the bridge's text without a translation.
    #[test]
    fn bridge_texts_translate_like_desktop() {
        let de = |kind, key, text| bridge_text_in(Some("de"), kind, key, text);
        let label = "fi.mau.telegram.login.phone_number/fi.mau.telegram.login.phone_number";
        // Desktop's German catalog leaves this label as its key, which keeps the bridge's text.
        assert_eq!(de("fieldLabel", label, "Phone number"), "Phone number", "an untranslated label");
        assert_eq!(de("flowName", "telegram/qr", "QR Code"), "Telegram/QR", "Desktop's German flow name");
        assert_eq!(de("error", "Invalid phone code", "Invalid phone code"), "Ungültiger Telefoncode");
        assert_eq!(de("flowName", "telegram/nope", "Nope"), "Nope", "a text without a translation stays");
        assert_eq!(de("instructions", "fi.mau.gmessages.complete", "Done"), "Done", "an ambiguous key stays");
        assert_eq!(bridge_text("flowName", "telegram/qr", "QR Code"), "QR Code", "English keeps the bridge's text");
    }
}
