// The OpenCode Go key: one field for Settings > Potato Bot. Same keychain field as the other keys (masked after saving,
// Replace and Remove, the full key is never shown again). Before it is stored, a shape that cannot be a key is told in one
// sentence; after it is stored the plan is asked once whether it accepts it ("Check key" asks again). The prefix is not a
// rule: the plan's docs do not give one (see lib/potato-go-models.ts).
import { useMemo, useState } from 'react';
import { plainError, type PotatoTransport } from '../../../lib/potato';
import { goKeyShapeProblem } from '../../../lib/potato-go-models';
import { secretsApi, type SecretsApi } from '../../../lib/secrets';
import { ApiKeyField } from '../ApiKeyField';
import { Btn, Note } from '../settings-ui';

export const GO_KEY_LABEL = 'OpenCode Go key';

export function GoKeyField({ transport, api = secretsApi }: { transport: PotatoTransport; api?: SecretsApi }) {
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);

  const check = async () => {
    setChecking(true);
    setResult(null);
    try {
      setResult({ ok: true, text: await transport.checkGoKey() });
    } catch (error) {
      setResult({ ok: false, text: plainError(error, 'Could not check the key.') });
    } finally {
      setChecking(false);
    }
  };

  // The shape is told before the key is stored; the plan's answer comes right after it is.
  const guarded = useMemo<SecretsApi>(
    () => ({
      ...api,
      set: async (name, value) => {
        const problem = goKeyShapeProblem(value);
        if (problem) throw new Error(problem);
        await api.set(name, value);
        void check();
      },
      remove: async (name) => {
        await api.remove(name);
        setResult(null);
      },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [api, transport],
  );

  return (
    <div className="st-key-go">
      <ApiKeyField
        label={GO_KEY_LABEL}
        name="potato_go_key"
        placeholder="Your OpenCode Go key"
        api={guarded}
        onFileChange={() => Promise.reject(new Error('Potato Bot keeps its key only in the system keychain, and this computer has none.'))}
      />
      <div className="st-key-check">
        <Btn disabled={checking} onClick={() => void check()}>
          {checking ? 'Checking…' : 'Check key'}
        </Btn>
        {result && <Note role={result.ok ? 'status' : 'alert'}>{result.text}</Note>}
      </div>
    </div>
  );
}
