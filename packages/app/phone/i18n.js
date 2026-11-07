// Every user-visible string lives here. Two full tables with identical keys.
// zh = Traditional Chinese (used for every zh-* locale).

export const en = {
  'app.name': 'Potato Bot',
  'common.cancel': 'Cancel',
  'common.close': 'Close',
  'common.back': 'Back',
  'common.try_again': 'Try again',
  'common.loading': 'Loading...',
  'pair.noqr': "Open the pairing QR on your PC (Settings > Phone) and scan it with this phone's camera.",
  'pair.working': 'Pairing with your PC...',
  'pair.title': 'Pair this phone',
  'pair.failed': 'This pairing code was already used or has run out, or this phone cannot reach your PC. On your PC open Settings > Phone and press Make a new code, then scan again.',
  'unreach.title': 'Cannot reach your PC',
  'unreach.text': 'This phone cannot reach your PC. Some Wi-Fi networks (guest, mesh, hotspot) block phones from reaching each other. On your PC open Settings > Phone and turn on Reach from outside your Wi-Fi, then scan the new QR.',
  'err.generic': 'Something went wrong. Try again.',
  'err.signed_out': 'This phone is not paired any more. Scan the QR on your PC again.',
  'err.rate': 'Too many tries. Wait a bit and try again.',
  'err.send': 'Could not send. Try again.',
  'err.answer': 'Could not send your answer. Try again.',
  'list.title': 'Bots',
  'list.empty': 'No bots yet. Make one on your PC.',
  'list.rooms': 'Rooms',
  'list.pause': 'Pause all',
  'list.paused_resume': 'Paused - Start bots again',
  'list.needs_you': 'Needs you',
  'list.no_messages': 'No messages yet',
  'list.settings': 'Settings',
  'list.unread': '{n} unread',
  'pc.status': 'PC: {name}',
  'pc.unknown': 'your PC',
  'state.working': 'Working',
  'state.needs_you': 'Needs you',
  'state.idle': 'Idle',
  'state.offline': 'Offline',
  'state.problem': 'Problem',
  'pause.title': 'Pause all bots?',
  'pause.body': 'Every bot stops working until you start them again.',
  'pause.ok': 'Pause all',
  'time.now': 'now',
  'time.m': '{n}m',
  'time.h': '{n}h',
  'chat.placeholder': 'Message',
  'chat.send': 'Send',
  'chat.photo': 'Add photo',
  'chat.photo_max': 'You can add up to 3 photos.',
  'chat.photo_remove': 'Remove photo',
  'chat.queued': 'Queued',
  'chat.typing': 'typing',
  'chat.empty': 'No messages yet. Say hi.',
  'chat.file': 'File: {name}',
  'chat.chart': 'Chart: {title}',
  'chat.you': 'You',
  'step.title_running': 'Working... {n} steps',
  'step.title_done': '{n} steps',
  'step.running': 'running',
  'step.done': 'done',
  'step.error': 'failed',
  'step.retry': 'trying again',
  'job.goal': 'Job',
  'job.spent': 'Spent {amount}',
  'job.stop': 'Stop',
  'job.stop_title': 'Stop this job?',
  'job.stop_body': 'The bot will stop working on it.',
  'job.stop_ok': 'Stop job',
  'card.allow_once': 'Allow once',
  'card.allow_job': 'Allow for this job',
  'card.deny': 'Deny',
  'card.why': 'Why: {why}',
  'card.connector': 'Connect {service} on your PC',
  'card.connector_hint': 'Open Settings on your PC to finish. Then pick here.',
  'card.login_pc': 'Sign in on your PC',
  'card.login_hint': 'Passwords never go through your phone.',
  'card.not_now': 'Not now',
  'card.mcp': 'Add tool: {name}',
  'room.placeholder': 'Message the room',
  'room.empty': 'No messages yet.',
  'room.typing': '{names} typing',
  'room.steps': '{bot}: {n} steps',
  'settings.title': 'Settings',
  'settings.phone': 'This phone',
  'settings.name': 'Name',
  'settings.pc': 'PC',
  'settings.notifications': 'Notifications',
  'settings.notif_hint': 'Get a buzz when a bot needs you. On iPhone, add this app to your Home Screen first.',
  'settings.notif_unsupported': 'This browser cannot show notifications.',
  'settings.notif_insecure': 'Notifications need a secure address (https). Use the Reach from outside link or your own https address.',
  'settings.notif_denied': 'Notifications are blocked. Allow them in your phone settings.',
  'settings.notif_failed': 'Could not turn on notifications.',
  'settings.language': 'Language follows your phone.',
  'settings.forget': 'Forget this phone',
  'settings.forget_title': 'Forget this phone?',
  'settings.forget_body': 'You will need to scan the QR on your PC again.',
  'settings.forget_ok': 'Forget'
};

export const zh = {
  'app.name': 'Potato Bot',
  'common.cancel': '取消',
  'common.close': '關閉',
  'common.back': '返回',
  'common.try_again': '再試一次',
  'common.loading': '載入中...',
  'pair.noqr': '請在電腦上開啟配對 QR（設定 > 手機），再用這支手機的相機掃描。',
  'pair.working': '正在與你的電腦配對...',
  'pair.title': '配對這支手機',
  'pair.failed': '這個配對碼已經用過或已過期，或是這支手機連不到你的電腦。請在電腦上開啟 設定 > 手機，按「產生新代碼」，再掃描一次。',
  'unreach.title': '連不到你的電腦',
  'unreach.text': '這支手機連不到你的電腦。有些 Wi-Fi（訪客、Mesh、熱點）不讓手機互相連線。請在電腦上開啟 設定 > 手機，打開「從 Wi-Fi 外連線」，再掃描新的 QR。',
  'err.generic': '出了點問題，請再試一次。',
  'err.signed_out': '這支手機已不在配對中。請再掃描電腦上的 QR。',
  'err.rate': '試太多次了，請稍等再試。',
  'err.send': '送不出去，請再試一次。',
  'err.answer': '回覆送不出去，請再試一次。',
  'list.title': '機器人',
  'list.empty': '還沒有機器人。請在電腦上建立一個。',
  'list.rooms': '聊天室',
  'list.pause': '全部暫停',
  'list.paused_resume': '已暫停 - 讓機器人繼續',
  'list.needs_you': '需要你',
  'list.no_messages': '還沒有訊息',
  'list.settings': '設定',
  'list.unread': '{n} 則未讀',
  'pc.status': '電腦：{name}',
  'pc.unknown': '你的電腦',
  'state.working': '工作中',
  'state.needs_you': '需要你',
  'state.idle': '閒置',
  'state.offline': '離線',
  'state.problem': '有問題',
  'pause.title': '要全部暫停嗎？',
  'pause.body': '所有機器人都會停止工作，直到你讓它們繼續。',
  'pause.ok': '全部暫停',
  'time.now': '剛剛',
  'time.m': '{n} 分',
  'time.h': '{n} 時',
  'chat.placeholder': '輸入訊息',
  'chat.send': '送出',
  'chat.photo': '加入照片',
  'chat.photo_max': '最多可加入 3 張照片。',
  'chat.photo_remove': '移除照片',
  'chat.queued': '排隊中',
  'chat.typing': '輸入中',
  'chat.empty': '還沒有訊息，打個招呼吧。',
  'chat.file': '檔案：{name}',
  'chat.chart': '圖表：{title}',
  'chat.you': '你',
  'step.title_running': '工作中... {n} 個步驟',
  'step.title_done': '{n} 個步驟',
  'step.running': '進行中',
  'step.done': '完成',
  'step.error': '失敗',
  'step.retry': '重試中',
  'job.goal': '工作',
  'job.spent': '已花費 {amount}',
  'job.stop': '停止',
  'job.stop_title': '要停止這項工作嗎？',
  'job.stop_body': '機器人會停止處理這項工作。',
  'job.stop_ok': '停止工作',
  'card.allow_once': '允許一次',
  'card.allow_job': '這項工作都允許',
  'card.deny': '拒絕',
  'card.why': '原因：{why}',
  'card.connector': '請在電腦上連接 {service}',
  'card.connector_hint': '請在電腦的設定中完成，再回這裡選擇。',
  'card.login_pc': '請在電腦上登入',
  'card.login_hint': '密碼不會經過你的手機。',
  'card.not_now': '現在不要',
  'card.mcp': '新增工具：{name}',
  'room.placeholder': '傳訊息到聊天室',
  'room.empty': '還沒有訊息。',
  'room.typing': '{names} 輸入中',
  'room.steps': '{bot}：{n} 個步驟',
  'settings.title': '設定',
  'settings.phone': '這支手機',
  'settings.name': '名稱',
  'settings.pc': '電腦',
  'settings.notifications': '通知',
  'settings.notif_hint': '機器人需要你時會通知你。iPhone 請先把這個 App 加到主畫面。',
  'settings.notif_unsupported': '這個瀏覽器無法顯示通知。',
  'settings.notif_insecure': '通知需要安全的網址（https）。請使用「從外面連線」的連結或你自己的 https 網址。',
  'settings.notif_denied': '通知被封鎖了。請在手機設定中允許。',
  'settings.notif_failed': '無法開啟通知。',
  'settings.language': '語言會跟隨你的手機。',
  'settings.forget': '忘記這支手機',
  'settings.forget_title': '要忘記這支手機嗎？',
  'settings.forget_body': '之後需要再掃描電腦上的 QR。',
  'settings.forget_ok': '忘記'
};

const tables = { en, zh };
let current = 'en';

/** Any zh-* locale (zh-TW, zh-Hant, zh-HK, zh-CN ...) -> 'zh' (Traditional). Else 'en'. */
export function pickLang(tag) {
  return /^zh(\b|[-_])/i.test(String(tag || '')) ? 'zh' : 'en';
}
export function setLang(code) { current = code === 'zh' ? 'zh' : 'en'; }
export function getLang() { return current; }

export function t(key, params) {
  let s = tables[current][key];
  if (s === undefined) s = en[key];
  if (s === undefined) return key;
  if (params) s = s.replace(/\{(\w+)\}/g, (m, k) => (params[k] === undefined ? m : String(params[k])));
  return s;
}

export function fmtMoney(nano) {
  const usd = Number(nano || 0) / 1e9;
  if (usd > 0 && usd < 0.01) return '<$0.01';
  return '$' + usd.toFixed(2);
}

export function fmtTime(ms, nowMs) {
  if (!ms) return '';
  const d = Math.max(0, nowMs - ms);
  if (d < 60000) return t('time.now');
  if (d < 3600000) return t('time.m', { n: Math.floor(d / 60000) });
  if (d < 86400000) return t('time.h', { n: Math.floor(d / 3600000) });
  try {
    return new Intl.DateTimeFormat(current === 'zh' ? 'zh-TW' : 'en', { month: 'short', day: 'numeric' }).format(new Date(ms));
  } catch { return ''; }
}
