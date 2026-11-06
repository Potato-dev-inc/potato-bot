// The chart a bot's `render_plot` puts in the chat: a bar, line or pie drawn here as an SVG from checked numbers.
// Every label is a React text child (escaped by React), never markup, and nothing is fetched or run.
import type { PotatoChart } from '../../../lib/potato';

const W = 320;
const H = 160;
const PAD = { left: 30, right: 8, top: 8, bottom: 22 };
/** Most x labels drawn under a bar or line chart; more points than this show every few labels. */
const MAX_X_LABELS = 8;
/** Bars show their number up to this many points. */
const MAX_BAR_VALUES = 12;

const shade = (i: number, n: number) => `hsl(0 0% ${n <= 1 ? 35 : 25 + Math.round((i * 55) / (n - 1))}%)`;
const num = (n: number) => (Number.isFinite(n) ? Math.round(n * 100) / 100 : 0);

/** A short number: 1200 -> 1.2k, 0.5 stays 0.5. */
export function compact(value: number): string {
  const a = Math.abs(value);
  const scaled = (n: number, unit: string) => `${value < 0 ? '-' : ''}${Math.round(n * 10) / 10}${unit}`;
  if (a >= 1e9) return scaled(a / 1e9, 'B');
  if (a >= 1e6) return scaled(a / 1e6, 'M');
  if (a >= 1e4) return scaled(a / 1e3, 'k');
  return String(Math.round(value * 100) / 100);
}

/** The value range with zero kept in view, never flat (a zero-height range would divide by zero). */
function range(values: number[], keepZero: boolean): [number, number] {
  let lo = Math.min(...values);
  let hi = Math.max(...values);
  if (keepZero) {
    lo = Math.min(lo, 0);
    hi = Math.max(hi, 0);
  }
  if (hi - lo === 0) {
    hi = lo + 1;
  }
  return [lo, hi];
}

function xLabels(labels: string[]): Array<[number, string]> {
  const step = Math.max(1, Math.ceil(labels.length / MAX_X_LABELS));
  return labels.map((l, i) => [i, l] as [number, string]).filter(([i]) => i % step === 0);
}

function Axes({ lo, hi, y }: { lo: number; hi: number; y: (v: number) => number }) {
  return (
    <>
      <line className="ck-potato-plot-axis" x1={PAD.left} x2={W - PAD.right} y1={y(Math.max(lo, Math.min(0, hi)))} y2={y(Math.max(lo, Math.min(0, hi)))} />
      <text className="ck-potato-plot-text" x={PAD.left - 4} y={y(hi) + 3} textAnchor="end">
        {compact(hi)}
      </text>
      <text className="ck-potato-plot-text" x={PAD.left - 4} y={y(lo)} textAnchor="end">
        {compact(lo)}
      </text>
    </>
  );
}

function Cartesian({ chart }: { chart: PotatoChart }) {
  const values = chart.points.map((p) => p.value);
  const [lo, hi] = range(values, chart.kind === 'bar');
  const plotW = W - PAD.left - PAD.right;
  const plotH = H - PAD.top - PAD.bottom;
  const y = (v: number) => num(PAD.top + plotH - ((v - lo) / (hi - lo)) * plotH);
  const n = chart.points.length;
  const slot = plotW / n;
  const cx = (i: number) => num(PAD.left + (chart.kind === 'bar' ? slot * (i + 0.5) : n === 1 ? plotW / 2 : (plotW * i) / (n - 1)));
  const base = y(Math.max(lo, Math.min(0, hi)));
  return (
    <svg className="ck-potato-plot-svg" viewBox={`0 0 ${W} ${H}`} role="img" aria-label={chart.title || 'Chart'}>
      <Axes lo={lo} hi={hi} y={y} />
      {chart.kind === 'bar' &&
        chart.points.map((p, i) => {
          const top = y(p.value);
          const barW = Math.max(1, Math.min(40, slot * 0.7));
          return (
            <g key={i}>
              <rect className="ck-potato-plot-bar" x={num(cx(i) - barW / 2)} y={Math.min(top, base)} width={num(barW)} height={num(Math.abs(base - top))} rx={1.5} />
              {n <= MAX_BAR_VALUES && (
                <text className="ck-potato-plot-text" x={cx(i)} y={p.value >= 0 ? Math.max(8, top - 3) : top + 10} textAnchor="middle">
                  {compact(p.value)}
                </text>
              )}
            </g>
          );
        })}
      {chart.kind === 'line' && (
        <>
          <path className="ck-potato-plot-line" d={chart.points.map((p, i) => `${i === 0 ? 'M' : 'L'}${cx(i)} ${y(p.value)}`).join(' ')} />
          {n <= 60 && chart.points.map((p, i) => <circle key={i} className="ck-potato-plot-dot" cx={cx(i)} cy={y(p.value)} r={2.2} />)}
        </>
      )}
      {xLabels(chart.points.map((p) => p.label)).map(([i, label]) => (
        <text key={i} className="ck-potato-plot-text" x={cx(i)} y={H - 7} textAnchor="middle">
          {label.length > 10 ? `${label.slice(0, 9)}…` : label}
        </text>
      ))}
    </svg>
  );
}

function Pie({ chart }: { chart: PotatoChart }) {
  const total = chart.points.reduce((sum, p) => sum + Math.max(0, p.value), 0);
  const shown = chart.points.map((p, i) => ({ ...p, i })).filter((p) => p.value > 0 && total > 0);
  const r = 62;
  const cx = W / 2;
  const cy = H / 2;
  let angle = -Math.PI / 2;
  const slices = shown.map((p, k) => {
    const share = p.value / total;
    const from = angle;
    angle += share * Math.PI * 2;
    const [x1, y1, x2, y2] = [cx + r * Math.cos(from), cy + r * Math.sin(from), cx + r * Math.cos(angle), cy + r * Math.sin(angle)];
    const d = `M${cx} ${cy} L${num(x1)} ${num(y1)} A${r} ${r} 0 ${share > 0.5 ? 1 : 0} 1 ${num(x2)} ${num(y2)} Z`;
    return { p, k, share, d };
  });
  return (
    <>
      <svg className="ck-potato-plot-svg" viewBox={`0 0 ${W} ${H}`} role="img" aria-label={chart.title || 'Chart'}>
        {slices.length === 1 ? (
          <circle className="ck-potato-plot-slice" cx={cx} cy={cy} r={r} style={{ fill: shade(0, 1) }} />
        ) : (
          slices.map((s) => <path key={s.p.i} className="ck-potato-plot-slice" d={s.d} style={{ fill: shade(s.k, slices.length) }} />)
        )}
      </svg>
      <ul className="ck-potato-plot-legend">
        {slices.map((s) => (
          <li key={s.p.i}>
            <span className="ck-potato-plot-swatch" style={{ background: shade(s.k, slices.length) }} />
            {s.p.label} {Math.round(s.share * 100)}%
          </li>
        ))}
      </ul>
    </>
  );
}

export function PlotCard({ chart }: { chart: PotatoChart }) {
  return (
    <figure className="ck-potato-plot" aria-label={chart.title ? `Chart: ${chart.title}` : 'Chart'}>
      {chart.title && <figcaption className="ck-potato-plot-title">{chart.title}</figcaption>}
      {chart.kind === 'pie' ? <Pie chart={chart} /> : <Cartesian chart={chart} />}
    </figure>
  );
}
