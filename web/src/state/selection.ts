// Selected entries of one folder, by path segment. The last toggled one anchors shift ranges.

export class Selection {
  private set = new Set<string>();
  private anchor: string | null = null;

  has(seg: string): boolean {
    return this.set.has(seg);
  }

  get size(): number {
    return this.set.size;
  }

  list(): string[] {
    return [...this.set];
  }

  toggle(seg: string): void {
    if (this.set.has(seg)) this.set.delete(seg);
    else this.set.add(seg);
    this.anchor = seg;
  }

  only(seg: string): void {
    this.set = new Set([seg]);
    this.anchor = seg;
  }

  /** Selects from the anchor to `to`, in listing order, replacing the rest. */
  range(to: string, order: string[]): void {
    const a = this.anchor === null ? -1 : order.indexOf(this.anchor);
    const b = order.indexOf(to);
    if (a < 0 || b < 0) {
      this.only(to);
      return;
    }
    const [lo, hi] = a < b ? [a, b] : [b, a];
    this.set = new Set(order.slice(lo, hi + 1));
  }

  all(order: string[]): void {
    this.set = new Set(order);
  }

  clear(): void {
    this.set.clear();
    this.anchor = null;
  }
}
