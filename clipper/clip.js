// Shared by the service worker (context menu) and the popup.

const ENDPOINT = 'http://127.0.0.1:47621/clip';

// The selected text, or a readable excerpt of the page (runs inside the page).
function readPage() {
  const selection = String(window.getSelection() || '').trim();
  if (selection) return selection;
  const root = document.querySelector('article, main, [role="main"]') || document.body;
  const text = (root?.innerText || '').replace(/\n{3,}/g, '\n\n').trim();
  return text.length > 800 ? `${text.slice(0, 800).trim()}…` : text;
}

export async function pageText(tabId) {
  try {
    const [result] = await chrome.scripting.executeScript({ target: { tabId }, func: readPage });
    return result?.result || '';
  } catch {
    // Pages like chrome:// or the Web Store can't be read; send just the link.
    return '';
  }
}

// POST a clip to Zima. Returns { ok, error }.
export async function sendClip({ title, url, text }) {
  const { token } = await chrome.storage.local.get('token');
  if (!token) return { ok: false, error: 'Paste the token from Zima → Settings → Web clipper into this extension first.' };
  try {
    const response = await fetch(ENDPOINT, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Zima-Token': token },
      body: JSON.stringify({ title, url, text }),
    });
    if (response.status === 401) return { ok: false, error: 'Wrong token: copy it again from Zima → Settings.' };
    if (!response.ok) return { ok: false, error: `Zima couldn't save it (error ${response.status}).` };
    return { ok: true };
  } catch {
    return { ok: false, error: "Couldn't reach Zima. Is it running?" };
  }
}

// A small confirmation shown on the page (runs inside the page).
function toast(message, ok) {
  const el = document.createElement('div');
  el.textContent = message;
  Object.assign(el.style, {
    position: 'fixed', bottom: '24px', left: '50%', transform: 'translateX(-50%)', zIndex: 2147483647,
    background: ok ? '#2b2926' : '#8c2a2a', color: '#faf8f5', padding: '10px 16px', borderRadius: '10px',
    font: '13px system-ui, sans-serif', boxShadow: '0 6px 18px rgba(0,0,0,.25)', transition: 'opacity .3s',
  });
  document.body.appendChild(el);
  setTimeout(() => { el.style.opacity = '0'; setTimeout(() => el.remove(), 300); }, 2200);
}

export async function showToast(tabId, message, ok) {
  try {
    await chrome.scripting.executeScript({ target: { tabId }, func: toast, args: [message, ok] });
  } catch {
    // Restricted page: fall back to the badge.
    await chrome.action.setBadgeText({ tabId, text: ok ? '✓' : '!' });
    await chrome.action.setBadgeBackgroundColor({ tabId, color: ok ? '#46a758' : '#e5484d' });
  }
}
