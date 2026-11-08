// Spaces (Labs `spaces_enabled`): the page. The rail on the left, the picked space's header on the right.
import { useEffect, useState } from 'react';
import '../../styles/cockpit-spaces.css';
import {
  spaceBlock,
  spaceChannelCreate,
  spaceDelete,
  spaceForget,
  spaceLeave,
  spaceReact,
  spaceRemoveMember,
  spaceShowOnline,
  spacesErrorText,
  usePresenceReporter,
  useSpace,
  useSpaces,
  type ReactionCode,
  type SpaceInvite,
} from '../../lib/spaces';
import { SpaceList } from './SpaceList';
import { ChannelList } from './ChannelList';
import { ChannelView } from './ChannelView';
import { NewChannelSheet } from './NewChannelSheet';
import { InviteSheet } from './InviteSheet';
import { OpenInvitesSheet } from './OpenInvitesSheet';
import { JoinSheet } from './JoinSheet';
import { JoinRequestSheet } from './JoinRequestSheet';
import { MemberList } from './MemberList';
import { BotPanel } from './BotPanel';
import { NewSpaceSheet } from './NewSpaceSheet';
import { Sheet } from '../cockpit/Sheet';
import { ConfirmSheet } from './ConfirmSheet';
import { MemberMenu } from './MemberMenu';
import { SpaceMenu } from './SpaceMenu';
import { MessageMenu } from './MessageMenu';

export const NO_ONE_ONLINE = 'No one else is online right now. New messages wait until someone is.';
export const OWNER_CANT_LEAVE = "You own this space. Owners can't leave in this version.";

export function SpacesPage() {
  const api = useSpaces(true);
  const view = api.view;
  const [selected, setSelected] = useState<string | null>(null);
  const [joiningOpen, setJoiningOpen] = useState(false);
  const [invite, setInvite] = useState<SpaceInvite | null>(null);
  /** The list of open invites (opens instead of making another one when the space already has one that works). */
  const [invitesOpen, setInvitesOpen] = useState(false);
  /** Invites made since this window opened. Their link and words are not stored anywhere readable, so only these can be shown again. */
  const [madeInvites, setMadeInvites] = useState<Record<string, SpaceInvite>>({});
  const [channelId, setChannelId] = useState<string | null>(null);
  const [addingChannel, setAddingChannel] = useState(false);
  const [creating, setCreating] = useState(false);
  const [peopleOpen, setPeopleOpen] = useState(false);
  const [infoOpen, setInfoOpen] = useState(false);
  const [botsOpen, setBotsOpen] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const [forgetting, setForgetting] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const { detail } = useSpace(selected);
  usePresenceReporter(true);
  // The checkbox follows the backend; after a click it shows the view the backend answered with.
  const [showOnlineSet, setShowOnlineSet] = useState<boolean | null>(null);
  const showOnline = showOnlineSet ?? view?.showOnline ?? true;
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);
  // The People panel (narrow windows) closes on Escape.
  useEffect(() => {
    if (!peopleOpen) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setPeopleOpen(false); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [peopleOpen]);
  useEffect(() => {
    if (!selected && view && view.spaces.length > 0) setSelected(view.spaces[0].id);
  }, [view, selected]);
  // Keep a channel picked while it is still active; otherwise open the first active one.
  useEffect(() => {
    if (!detail) return;
    const active = detail.channels.filter((c) => !c.archived);
    if (!channelId || !active.some((c) => c.id === channelId)) setChannelId(active[0]?.id ?? null);
  }, [detail, channelId]);
  const current = detail?.channels.find((c) => c.id === channelId && !c.archived) ?? null;
  const joining = view?.joining[0] ?? null;
  const request = view?.requests[0] ?? null;
  const onNew = () => setCreating(true);
  const openInvites = (view?.invites ?? []).filter((i) => i.space === selected && i.state === 'open' && i.expiresAtMs > now);
  const makeInvite = async (space: string) => {
    const i = await api.invite(space);
    if (i) {
      setMadeInvites((m) => ({ ...m, [i.id]: i }));
      setInvitesOpen(false);
      setInvite(i);
    }
  };
  const active = detail?.state === 'active';
  /** Run one moderation call; a failure shows as a plain line under the page. */
  const run = async (call: Promise<unknown>) => {
    try {
      await call;
      setActionError(null);
    } catch (e) {
      setActionError(spacesErrorText(e));
    }
  };

  return (
    <div className="ck-spaces">
      {(view ? view.spaces.length === 0 : api.error !== null) && !joiningOpen ? (
        <section className="ck-spaces-empty">
          <div className="ck-spaces-empty-card">
            <h1 className="ck-title">Spaces</h1>
            <p className="ck-sub">Chat with friends in small groups, computer to computer. No server keeps your messages.</p>
            <div className="ck-spaces-empty-actions">
              <button type="button" className="ck-btn ck-btn--primary" onClick={onNew}>
                New space
              </button>
              <button type="button" className="ck-btn ck-btn--secondary" onClick={() => setJoiningOpen(true)}>
                Join a space
              </button>
            </div>
            <p className="ck-spaces-group-foot">Messages live on members' computers. If everyone is offline, new messages wait.</p>
            {view && !view.online && (
              <p className="ck-spaces-hint ck-spaces-offline" role="status">
                You're offline. New messages will go out when you're back online.
              </p>
            )}
            {!view && api.error && <p className="ck-spaces-hint" role="status">{api.error}</p>}
          </div>
        </section>
      ) : (
        <div className="ck-spaces-layout">
          <SpaceList
            spaces={view?.spaces ?? []}
            selected={selected}
            onSelect={setSelected}
            onNew={onNew}
            onJoin={() => setJoiningOpen(true)}
          />
          <main className="ck-spaces-main">
            {view && !view.online && (
              <p className="ck-spaces-hint ck-spaces-offline" role="status">
                You're offline. New messages will go out when you're back online.
              </p>
            )}
            {!detail ? (
              <p className="ck-sub">Pick a space on the left.</p>
            ) : (
              <header className="ck-spaces-head">
                <div className="ck-spaces-head-main">
                  <h1 className="ck-title">{detail.name}</h1>
                  {!detail.anyoneOnline && (
                    <span className="ck-spaces-note" title={NO_ONE_ONLINE}>{NO_ONE_ONLINE}</span>
                  )}
                  {detail.relayed && <span className="ck-spaces-note">Connected through a relay. Messages stay encrypted.</span>}
                </div>
                <div className="ck-spaces-head-actions">
                  <button type="button" className="ck-spaces-btn ck-spaces-people-toggle" aria-expanded={peopleOpen}
                    onClick={() => setPeopleOpen((o) => !o)}>
                    People
                  </button>
                  {active && (
                    <button type="button" className="ck-spaces-btn" onClick={() => setBotsOpen(true)}>
                      Bots
                    </button>
                  )}
                  <button type="button" className="ck-spaces-btn" onClick={() => setInfoOpen(true)}>
                    Space info
                  </button>
                  {detail.isOwner && active && (
                    <button
                      type="button"
                      className="ck-spaces-btn"
                      onClick={() => {
                        if (openInvites.length > 0) setInvitesOpen(true);
                        else void makeInvite(detail.id);
                      }}
                    >
                      Invite people
                    </button>
                  )}
                  {active && (
                    <SpaceMenu canLeave={!detail.isOwner} leaveBlocked={detail.isOwner ? OWNER_CANT_LEAVE : undefined}
                      onLeave={() => setLeaving(true)} />
                  )}
                </div>
              </header>
            )}
            {detail && !active && (
              <div className="ck-spaces-banner" role="status">
                <span>
                  {detail.state === 'removed'
                    ? `You were removed from ${detail.name}. You can still read what you have here.`
                    : `You left ${detail.name}.`}
                </span>
                <button type="button" className="ck-spaces-btn" onClick={() => setForgetting(true)}>
                  Delete my copy
                </button>
              </div>
            )}
            {detail && (
              <div className="ck-spaces-body">
                <ChannelList
                  channels={detail.channels}
                  selected={current?.id ?? null}
                  isOwner={detail.isOwner}
                  onSelect={setChannelId}
                  onNew={() => setAddingChannel(true)}
                />
                {current && (
                  <ChannelView
                    key={current.id}
                    space={detail.id}
                    channel={current}
                    readOnly={!active}
                    bots={detail.bots}
                    botsOff={detail.botRules?.channels != null && !detail.botRules.channels.includes(current.id)}
                    menu={(m, reply) => (
                      <MessageMenu
                        message={m}
                        canDelete={active && (m.isMine || detail.isOwner)}
                        onReply={reply}
                        onReact={(code: ReactionCode) => {
                          const mine = m.reactions.find((r) => r.code === code)?.mine ?? false;
                          void run(spaceReact(detail.id, m.id, code, !mine));
                        }}
                        onDelete={() => void run(spaceDelete(detail.id, m.id))}
                      />
                    )}
                    onReact={(m, code, on) => void run(spaceReact(detail.id, m.id, code as ReactionCode, on))}
                  />
                )}
                <MemberList
                  members={detail.members}
                  open={peopleOpen}
                  bots={detail.bots}
                  menu={(member) => (
                    <MemberMenu
                      member={member}
                      spaceName={detail.name}
                      canRemove={detail.isOwner && active && member.role !== 'owner'}
                      onRemove={() => void run(spaceRemoveMember(detail.id, member.member))}
                      onBlock={(on) => void run(spaceBlock(detail.id, member.member, on))}
                    />
                  )}
                />
              </div>
            )}
            {(actionError ?? api.error) && (
              <p className="ck-spaces-hint" role="alert">
                {actionError ?? api.error}
              </p>
            )}
          </main>
        </div>
      )}
      {addingChannel && detail && (
        <NewChannelSheet
          onCreate={async (name) => {
            try {
              await spaceChannelCreate(detail.id, name);
              return null;
            } catch (e) {
              return spacesErrorText(e);
            }
          }}
          onClose={() => setAddingChannel(false)}
        />
      )}
      {creating && (
        <NewSpaceSheet
          error={api.error}
          onCreate={async (name) => {
            const id = await api.create(name);
            if (id) setSelected(id);
            return id;
          }}
          onClose={() => setCreating(false)}
        />
      )}
      {botsOpen && detail && (
        <BotPanel space={detail.id} spaceName={detail.name} channels={detail.channels} onClose={() => setBotsOpen(false)} />
      )}
      {infoOpen && detail && (
        <Sheet labelledBy="ck-space-info-title" role="dialog" onClose={() => setInfoOpen(false)} className="ck-spaces-sheet">
          <h2 id="ck-space-info-title" className="ck-spaces-sheet-title">
            Space info
          </h2>
          <div className="ck-spaces-sheet-scroll">
          <div className="ck-spaces-group">
            <div className="ck-spaces-card">
              <p className="ck-spaces-sheet-body">{detail.name}</p>
              <p className="ck-spaces-sheet-body">
                {detail.members.length} {detail.members.length === 1 ? 'person' : 'people'}
              </p>
            </div>
            <p className="ck-spaces-group-foot">
              Messages live on members' computers. If everyone is offline, new messages wait. This computer keeps the last 90
              days, up to 200 MB per space.
            </p>
            {(detail.leavePending || detail.state === 'left') && (
              <p className="ck-spaces-group-foot">
                Until the owner's computer is online, people who left can still read new messages.
              </p>
            )}
          </div>
          <div className="ck-spaces-group">
          <div className="ck-spaces-card">
          <label className="ck-spaces-check">
            <input
              type="checkbox"
              className="ck-spaces-switch"
              checked={showOnline}
              onChange={(e) => {
                const on = e.target.checked;
                spaceShowOnline(on).then(
                  (v) => setShowOnlineSet(v.showOnline),
                  (err: unknown) => setActionError(spacesErrorText(err)),
                );
              }}
            />{' '}
            Show when I'm online
          </label>
          </div>
          <p className="ck-spaces-group-foot">This applies to all spaces on this computer. When it is off, others see you as offline.</p>
          </div>
          </div>
          <div className="ck-sheet-actions">
            <button type="button" className="ck-btn ck-btn--primary" onClick={() => setInfoOpen(false)}>
              Close
            </button>
          </div>
        </Sheet>
      )}
      {leaving && detail && (
        <ConfirmSheet
          title={`Leave ${detail.name}?`}
          body="You will stop getting new messages. Your copy stays on this computer until you delete it."
          confirmLabel="Leave"
          onClose={() => setLeaving(false)}
          onConfirm={() => {
            setLeaving(false);
            void run(spaceLeave(detail.id));
          }}
        />
      )}
      {forgetting && detail && (
        <ConfirmSheet
          title={`Delete your copy of ${detail.name}?`}
          body="This removes its messages and files from this computer. It can't be undone."
          confirmLabel="Delete my copy"
          onClose={() => setForgetting(false)}
          onConfirm={() => {
            setForgetting(false);
            setSelected(null);
            void run(spaceForget(detail.id));
          }}
        />
      )}
      {invitesOpen && detail && !invite && (
        <OpenInvitesSheet
          spaceName={detail.name}
          invites={openInvites}
          known={madeInvites}
          nowMs={now}
          error={api.error}
          onShow={setInvite}
          onCancelInvite={(i) => void api.cancelInvite(detail.id, i.id)}
          onNew={() => void makeInvite(detail.id)}
          onClose={() => setInvitesOpen(false)}
        />
      )}
      {invite && detail && (
        <InviteSheet
          spaceName={detail.name}
          invite={invite}
          nowMs={now}
          onCancelInvite={() => {
            void api.cancelInvite(invite.space, invite.id);
            setInvite(null);
          }}
          onClose={() => setInvite(null)}
        />
      )}
      {(joiningOpen || (joining !== null && joining.state !== 'joined')) && (
        <JoinSheet
          joining={joining}
          error={api.error}
          onJoin={(text, words) => void api.join(text, words)}
          onCancel={(space) => {
            if (space) void api.cancelJoin(space);
            setJoiningOpen(false);
          }}
          onOpenSpace={(space) => {
            setSelected(space);
            setJoiningOpen(false);
          }}
        />
      )}
      <JoinRequestSheet request={request} onAnswer={(s, r, allow) => void api.answer(s, r, allow)} />
    </div>
  );
}
