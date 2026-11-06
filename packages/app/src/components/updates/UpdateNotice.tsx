// A quiet notice in a corner: "Update available: 1.0.3" with an Update button. Nothing installs until the button is pressed;
// with a job running it asks once more first. A failed update says why in a plain sentence and the app keeps running.
import type { AppUpdateState } from './useAppUpdate';
import { runningJobWarning } from '../../lib/app-update';
import '../../styles/app-update.css';

export function UpdateNoticeView({ state }: { state: AppUpdateState }) {
  const { phase, offer } = state;
  if (phase === 'idle' || phase === 'none' || phase === 'checking') return null;
  return (
    <div className="app-update-notice" role="status" data-phase={phase}>
      {phase === 'available' && offer && (
        <>
          <span>Update available: {offer.version}</span>
          <button type="button" className="app-update-btn" onClick={() => void state.update()}>
            Update
          </button>
        </>
      )}
      {phase === 'warn' && (
        <>
          <span>{runningJobWarning(state.busyBots)}</span>
          <button type="button" className="app-update-btn" onClick={() => void state.update()}>
            Update anyway
          </button>
          <button type="button" className="app-update-btn app-update-btn--quiet" onClick={state.dismissWarning}>
            Not now
          </button>
        </>
      )}
      {phase === 'installing' && <span>Updating. Potato Bot will close and open again.</span>}
      {phase === 'error' && (
        <>
          <span>{state.error}</span>
          {offer && (
            <button type="button" className="app-update-btn" onClick={() => void state.update()}>
              Try again
            </button>
          )}
        </>
      )}
    </div>
  );
}
