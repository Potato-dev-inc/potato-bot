/** fishd example: 12 unprompted replies in 30 minutes. Counted per bot per room. */
export const SILENCE_CAP = 12;
export const SILENCE_WINDOW_MS = 30 * 60_000;
/** After this bot spoke unprompted, wait this long before speaking unprompted again. */
export const COOLDOWN_MS = 20_000;

export class SilenceBudget {
  private spentAt: number[] = [];
  /** "budget" = cap reached; "cooldown" = spoke too recently; null = may speak. */
  blocked(now: number): "budget" | "cooldown" | null {
    this.spentAt = this.spentAt.filter((t) => now - t < SILENCE_WINDOW_MS);
    const last = this.spentAt[this.spentAt.length - 1];
    if (last !== undefined && now - last < COOLDOWN_MS) return "cooldown";
    if (this.spentAt.length >= SILENCE_CAP) return "budget";
    return null;
  }
  /** Call only for unprompted replies. Addressed replies never spend budget. */
  spend(now: number): void { this.spentAt.push(now); }
  spentCount(now: number): number { this.blocked(now); return this.spentAt.length; }
}
