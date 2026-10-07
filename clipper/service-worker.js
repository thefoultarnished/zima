import { pageText, sendClip, showToast } from './clip.js';

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: 'clip-page', title: 'Send page to Zima', contexts: ['page'] });
  chrome.contextMenus.create({ id: 'clip-selection', title: 'Send selection to Zima', contexts: ['selection'] });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (!tab?.id) return;
  const text = info.menuItemId === 'clip-selection' ? (info.selectionText || '') : await pageText(tab.id);
  const result = await sendClip({ title: tab.title || '', url: tab.url || info.pageUrl || '', text });
  await showToast(tab.id, result.ok ? 'Sent to Zima' : result.error, result.ok);
});
