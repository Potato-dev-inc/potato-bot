// Whether "Apps from your PC" shows: the Labs switch `potato_lend_app_enabled`, Potato Bot too, and a Windows PC.
// Read from the settings file once, then again whenever the settings change. Off until the first read says otherwise.
import { useEffect, useState } from 'react';
import { loadSettings } from '../../../lib/settings';
import { subscribeSettingsRefresh } from '../../../lib/settings-sync';
import { LEND_FLAG, isWindowsRuntime } from '../../../lib/potatoLend';

export function useLendOn(): boolean {
  const [on, setOn] = useState(false);
  useEffect(() => {
    let live = true;
    const read = () => {
      loadSettings().then(
        (settings) => {
          const flags = settings as { potato_bot_enabled?: boolean } & Record<string, unknown>;
          if (live) setOn(flags.potato_bot_enabled === true && flags[LEND_FLAG] === true && isWindowsRuntime());
        },
        () => {
          if (live) setOn(false);
        },
      );
    };
    read();
    const unsubscribe = subscribeSettingsRefresh(read);
    return () => {
      live = false;
      unsubscribe();
    };
  }, []);
  return on;
}
