import { ZH_TW_POTATO } from './zh-TW.potato';

// Traditional Chinese (Taiwan) for the app's own words. The key is the English sentence exactly as the screen has it; the value is
// the same meaning in Traditional Chinese. A sentence that is not here is shown in English. Keep every {name} placeholder.
export const ZH_TW: Readonly<Record<string, string>> = {
  ...ZH_TW_POTATO,
  // Settings window
  Settings: '設定',
  Search: '搜尋',
  'Search settings': '搜尋設定',
  'Settings sections': '設定分類',
  'Search results': '搜尋結果',
  'Coming later': '即將推出',
  'Not built yet. None of these do anything today.': '尚未完成，目前都還不能使用。',
  'Not available yet': '尚未提供',
  'Loading settings': '正在載入設定',
  'Close settings': '關閉設定',
  Back: '返回',
  Saved: '已儲存',
  Copy: '複製',
  Language: '語言',
  'Follow the system': '跟隨系統',
  'Potato Bot': 'Potato Bot',
  'Bots, their brains, helpers and their computer.': '機器人、它們的大腦、小幫手和它們的電腦。',
  Phone: '手機',
  'Use your bots from your phone.': '用手機使用你的機器人。',
  'No settings match “{query}”.': '找不到符合「{query}」的設定。',
};
