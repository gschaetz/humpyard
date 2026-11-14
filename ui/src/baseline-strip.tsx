import type { Stats } from "./api/client";
import { describeBaseline } from "./baseline";

/**
 * What the traffic would have cost at the chosen baseline, in words, above the chart: the sentence, then what the
 * figure is and leaves out. Everything in it comes from the answer and is shown as text. It draws nothing when the
 * answer carries no baseline.
 */
export function BaselineStrip(props: { stats: Stats }) {
  const { baseline, summary } = props.stats;
  if (baseline === undefined || summary === undefined) return null;
  const { headline, notes } = describeBaseline(summary, baseline);
  return (
    <section class="baseline-strip" aria-label="What this traffic would have cost">
      <p class="baseline-headline">{headline}</p>
      {notes.map((note) => (
        <p key={note} class="muted baseline-note">{note}</p>
      ))}
    </section>
  );
}
