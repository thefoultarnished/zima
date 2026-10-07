import { pageText, sendClip } from './clip.js';

const status = document.getElementById('status');
const sendButton = document.getElementById('send-button');
const tokenInput = document.getElementById('token');

function show(message, isError = false) {
  status.textContent = message;
  status.className = isError ? 'error' : '';
}

async function init() {
  const { token } = await chrome.storage.local.get('token');
  if (token) tokenInput.placeholder = 'Saved (paste to replace)';
  else show('First, paste the token from Zima.');
}

document.getElementById('save').addEventListener('click', async () => {
  const token = tokenInput.value.trim();
  if (!token) return;
  await chrome.storage.local.set({ token });
  tokenInput.value = '';
  tokenInput.placeholder = 'Saved (paste to replace)';
  show('Token saved.');
});

sendButton.addEventListener('click', async () => {
  sendButton.disabled = true;
  show('Sending…');
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    const text = tab?.id ? await pageText(tab.id) : '';
    const result = await sendClip({ title: tab?.title || '', url: tab?.url || '', text });
    if (result.ok) {
      show('Sent to Zima ✓');
      setTimeout(() => window.close(), 900);
    } else {
      show(result.error, true);
    }
  } finally {
    sendButton.disabled = false;
  }
});

init();
