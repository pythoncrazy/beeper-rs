//! The scripts that Desktop's `mapBrowserLogin` of some platforms runs in the login window (load-stores of
//! Desktop 4.3.308), as they are.

/// X's `an`: hides the Google and Apple sign-in buttons.
pub const TWITTER: &str = r#"
  try {
    const style = document.createElement('style');
    style.textContent = '[data-testid="google_sign_in_container"], [data-testid="apple_sign_in_button"], [data-testid="apple_sign_in_button"] + div { display: none !important; }';
    document.head.appendChild(style);
  } catch{}
"#;

/// Slack's `cn`: hides the Google and Apple sign-in buttons, and opens the workspace links in the same window.
pub const SLACK: &str = r#"
  try {
    const style = document.createElement('style');
    style.textContent = '.c-google_login, .c-apple_login, .c-horizontal_content_rule { display: none !important; }';
    document.head.appendChild(style);
  } catch{}

      window.__overrideWorkspaceLink = window.__overrideWorkspaceLink || function () {
        const url = window.location.href
        if (!url.includes('signin.findWorkspaces') && !url.includes('signin#/workspaces')) return

        const elements = document.querySelectorAll('.p-workspaces_list__link')
        elements.forEach((element) => {
          // Do not open links in a new window
          element.target = ''
        })
      }

      window.__addEventsListeners = window.__addEventsListeners || function () {
        const observer = new MutationObserver(() => window.__overrideWorkspaceLink())
        const container = document.documentElement || document.body
        observer.observe(container, { childList: true, subtree: true })
      }

  window.__addEventsListeners()
  window.__overrideWorkspaceLink()
  "#;

/// Facebook's `Up` for the `messenger` flow: goes on to Messenger after a Facebook login.
pub const MESSENGER: &str = r#"
if (!window.isBeepinated) {
  window.isBeepinated = true
  
const declineButton = document.querySelector('button:has(+ #allow_button)')
if (declineButton) {
  console.log('[BrowserAuth] GDPR banner shown — declining optional cookies');
  declineButton.click();
}


  const bLog = (...msg) => console.log('[BrowserAuth]', ...msg);
  bLog('Loaded Messenger login script from', window.location.href);
  if (window.location.href === 'https://www.facebook.com/checkpoint/?next=https%3A%2F%2Fwww.facebook.com%2F') {
    bLog('Hit Facebook checkpoint, redirecting to Messenger');
    window.location.href = 'https://www.facebook.com/checkpoint/?next=https%3A%2F%2Fwww.messenger.com%2F';
  } else if (window.location.hostname === 'www.facebook.com') {
    bLog('Loaded on Facebook, expecting Messenger, will redirect after login');
    const beepAllDayAllNight = setInterval(() => {
      if (+require('CurrentUserInitialData')?.USER_ID) {
        bLog('Logged in on Facebook, redirecting to Messenger');
        clearInterval(beepAllDayAllNight)
        window.location.href = 'https://www.messenger.com/?no_redirect=true'
      }
    }, 100)
  }
}
"#;

/// Facebook's `Fp` for the other flows: closes the window after a Facebook login.
pub const FACEBOOK: &str = r#"

const declineButton = document.querySelector('button:has(+ #allow_button)')
if (declineButton) {
  console.log('[BrowserAuth] GDPR banner shown — declining optional cookies');
  declineButton.click();
}

let successfulLogin = false

if (window.location.hostname === 'www.facebook.com') {
  const interval = setInterval(() => {
    if (+require('CurrentUserInitialData')?.USER_ID) {
      clearInterval(interval)
      successfulLogin = true
      setTimeout(() => window.close(), 500)
    }
  }, 100)
}
"#;

/// Instagram's `ln`: closes the window after the login.
pub const INSTAGRAM: &str = r#"
  let successfulLogin = false
  setInterval(() => {
    if (
      window.location.hostname === 'www.instagram.com' &&
      require?.('PolarisViewer')?.id
    ) {
      successfulLogin = true
      setTimeout(() => window.close(), 500)
    }
  }, 200)
"#;

/// LinkedIn's `dn`: hides the Google, Apple and passkey sign-in.
pub const LINKEDIN: &str = r#"
  try {
    if (!window.__beeperHideLinkedInSSO) {
      window.__beeperHideLinkedInSSO = true;
      const hideSSO = () => {
        if (!/^\/(login|checkpoint|uas)/.test(location.pathname)) return;
        const starts = [
          ...document.querySelectorAll('iframe[src*="accounts.google.com/gsi"]'),
          ...[...document.querySelectorAll('button')].filter(b => b.textContent.length < 60 && /\b(apple|passkey)\b/i.test(b.textContent)),
        ];
        for (const start of starts) {
          let box = start;
          while (box.parentElement && !box.parentElement.querySelector('input')) box = box.parentElement;
          if (!box.parentElement || box.style.display === 'none') continue;
          box.style.setProperty('display', 'none', 'important');
          const divider = box.nextElementSibling;
          if (divider && !divider.querySelector('input') && divider.textContent.trim().toLowerCase() === 'or') {
            divider.style.setProperty('display', 'none', 'important');
          }
        }
      };
      // runJSOnNavigate can fire on a freshly committed document, so wait for <head> instead of failing once
      const start = () => {
        if (!document.documentElement || !document.head) return setTimeout(start, 50);
        const style = document.createElement('style');
        style.textContent = '.alternate-signin-container { display: none !important; }';
        document.head.appendChild(style);
        hideSSO();
        new MutationObserver(hideSSO).observe(document.documentElement, { childList: true, subtree: true });
      };
      start();
    }
  } catch {}
"#;
