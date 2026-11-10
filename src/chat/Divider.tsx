/**
 * A quiet full-width line in the history, with kelpie's words in the middle:
 * where a new session began, such as a switch of harness.
 */
export function Divider({ text }: { text: string }) {
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      <hr className="flex-1 border-border" />
      <span>{text}</span>
      <hr className="flex-1 border-border" />
    </div>
  );
}
