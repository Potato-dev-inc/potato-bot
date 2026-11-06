import { tauriUpdateTransport, type UpdateTransport } from '../../lib/app-update';
import { UpdateNoticeView } from './UpdateNotice';
import { useAppUpdate } from './useAppUpdate';

/** Mounted once in the app: checks quietly and shows the notice only when there is something to say. */
export function AppUpdateHost({ transport = tauriUpdateTransport }: { transport?: UpdateTransport }) {
  const state = useAppUpdate(transport);
  return <UpdateNoticeView state={state} />;
}
