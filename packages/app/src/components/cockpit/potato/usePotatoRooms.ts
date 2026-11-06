// The bot groups the list shows (plan 12, slice G9). Read through the transport while the group switch is on, then
// read again whenever the app says a group changed. Off: nothing is read and the list is empty.
import { useCallback, useEffect, useState } from 'react';
import { isRoomGone, roomErrorText, tauriRoomsTransport, type RoomLine, type RoomsTransport, type RoomView } from '../../../lib/potato-rooms';

export function usePotatoRooms(transport: RoomsTransport = tauriRoomsTransport, enabled: boolean): { rooms: RoomView[]; refresh: () => void; /** The list has been read at least once since the switch went on. */ loaded: boolean } {
  const [rooms, setRooms] = useState<RoomView[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [tick, setTick] = useState(0);
  const refresh = useCallback(() => setTick((n) => n + 1), []);

  useEffect(() => {
    if (!enabled) {
      setRooms([]);
      setLoaded(false);
      return;
    }
    let live = true;
    transport.list().then(
      (next) => {
        if (!live) return;
        setRooms(next);
        setLoaded(true);
      },
      () => live && setRooms([]),
    );
    return () => {
      live = false;
    };
  }, [transport, enabled, tick]);

  useEffect(() => {
    if (!enabled) return;
    let gone = false;
    let unlisten: (() => void) | undefined;
    transport.onChanged(() => refresh()).then(
      (fn) => {
        if (gone) fn();
        else unlisten = fn;
      },
      () => {},
    );
    return () => {
      gone = true;
      unlisten?.();
    };
  }, [transport, enabled, refresh]);

  return { rooms, refresh, loaded };
}

/** How many lines the thread reads. */
export const ROOM_LINES_LIMIT = 200;

/**
 * The lines of one group, oldest first. Read when the group opens and again whenever the app says that group (or any
 * group) changed. `gone` is true when the app says the group no longer exists; `problem` is a plain sentence for any
 * other failure.
 */
export function useRoomLines(transport: RoomsTransport, roomId: string): { lines: RoomLine[]; gone: boolean; problem: string | null; loaded: boolean } {
  const [state, setState] = useState<{ roomId: string; lines: RoomLine[]; gone: boolean; problem: string | null; loaded: boolean }>({ roomId, lines: [], gone: false, problem: null, loaded: false });
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let live = true;
    transport.lines(roomId, ROOM_LINES_LIMIT).then(
      (lines) => live && setState({ roomId, lines, gone: false, problem: null, loaded: true }),
      (error) => live && setState((was) => ({ roomId, lines: was.roomId === roomId ? was.lines : [], gone: isRoomGone(error), problem: roomErrorText(error, 'Could not read the group.'), loaded: true })),
    );
    return () => {
      live = false;
    };
  }, [transport, roomId, tick]);

  useEffect(() => {
    let gone = false;
    let unlisten: (() => void) | undefined;
    transport.onChanged((changed) => {
      if (changed === null || changed === roomId) setTick((n) => n + 1);
    }).then(
      (fn) => {
        if (gone) fn();
        else unlisten = fn;
      },
      () => {},
    );
    return () => {
      gone = true;
      unlisten?.();
    };
  }, [transport, roomId]);

  const current = state.roomId === roomId;
  return { lines: current ? state.lines : [], gone: current && state.gone, problem: current ? state.problem : null, loaded: current && state.loaded };
}
