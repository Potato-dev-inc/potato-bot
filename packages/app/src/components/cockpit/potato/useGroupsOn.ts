// Whether bot groups are on: the Labs switch `potato_groups_enabled` and Potato Bot too. Read from the settings file
// once, then again whenever the settings change. Off until the first read says otherwise.
import { useEffect, useState } from 'react';
import { loadSettings } from '../../../lib/settings';
import { subscribeSettingsRefresh } from '../../../lib/settings-sync';

export function useGroupsOn(): boolean {
  const [on, setOn] = useState(false);
  useEffect(() => {
    let live = true;
    const read = () => {
      loadSettings().then(
        (settings) => {
          const flags = settings as { potato_bot_enabled?: boolean; potato_groups_enabled?: boolean };
          if (live) setOn(flags.potato_bot_enabled === true && flags.potato_groups_enabled === true);
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
