import { create } from "zustand";
import {
  type AgentSummary,
  type AgentView,
  commands,
  type Frontmatter,
  type ResolvedAgent,
} from "@/bindings";

/** What the editor is changing: the file's own keys and body. */
export type Draft = { frontmatter: Frontmatter; body: string };

/**
 * The keys a form field can hold. `body` is the switch's, not a field, and a
 * reviewer's `paths` and `second_look` have no row: they pass through as they stand.
 */
export type FieldKey = Exclude<
  keyof Frontmatter,
  "body" | "paths" | "second_look"
>;

type Agents = {
  /** The agents the config folder holds, `null` until read. */
  agents: AgentSummary[] | null;
  /** The agent as it was read, `null` while none is open. */
  open: AgentView | null;
  /** What the editor holds: the file's own keys and body, edited and not yet saved. */
  draft: Draft | null;
  /** The draft as a session would get it, `null` until the loader has answered. */
  preview: ResolvedAgent | null;
  /** What the loader refuses in the draft, shown with the preview. */
  previewError: string | null;
  /** The last failure to read, save or make an agent. A refused save is here. */
  error: string | null;
  /** Whether the draft was saved and not changed since. */
  saved: boolean;
  /** Read the list of agents. */
  loadList: () => Promise<void>;
  /** Read one agent into the editor, with its draft the file as it is. */
  openAgent: (name: string) => Promise<void>;
  /** Set one key of the draft; `null` clears it. */
  setKey: (key: FieldKey, value: string | null) => void;
  /** Set the draft's body. */
  setBody: (body: string) => void;
  /** Whether the draft puts its body after its parent's. */
  setAppend: (append: boolean) => void;
  /** Write the draft. A file the loader refuses is not written, and the draft stays. */
  save: () => Promise<boolean>;
  /** Make an agent that builds on `parent`, then list again. */
  create: (name: string, parent: string | null) => Promise<boolean>;
  /** Close the open agent. */
  close: () => void;
  /** Forget everything. For tests. */
  reset: () => void;
};

const empty = {
  agents: null,
  open: null,
  draft: null,
  preview: null,
  previewError: null,
  error: null,
  saved: false,
} satisfies Partial<Agents>;

/** How long a pause in typing is before the loader is asked about the draft. */
const PAUSE_MS = 200;

/**
 * The keys as a file holds them: a key left empty in the form is no key. The
 * draft keeps what was typed, so a field being rewritten is not refilled with
 * what it inherits halfway.
 */
export function clean(front: Frontmatter): Frontmatter {
  const keep = (value: string | null) =>
    value === null || value.trim() === "" ? null : value;
  return {
    extends: keep(front.extends),
    role: keep(front.role),
    harness: keep(front.harness),
    model: keep(front.model),
    effort: keep(front.effort),
    body: keep(front.body),
    // A reviewer's keys have no row in the form, so they are kept as read
    paths: front.paths,
    second_look: front.second_look,
  };
}

export const useAgents = create<Agents>((set, get) => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  /** Counts the questions to the loader, so an old answer is dropped. */
  let asked = 0;

  /** Ask the loader what the draft makes, now. */
  async function ask() {
    const { open, draft } = get();
    if (open === null || draft === null) return;
    const mine = ++asked;
    let answer: { preview: ResolvedAgent | null; previewError: string | null };
    try {
      const done = await commands.agentPreview(
        open.name,
        clean(draft.frontmatter),
        draft.body,
      );
      answer =
        done.status === "ok"
          ? { preview: done.data, previewError: null }
          : { preview: get().preview, previewError: done.error };
    } catch (error) {
      answer = { preview: get().preview, previewError: String(error) };
    }
    if (mine === asked) set(answer);
  }

  function edited(draft: Draft) {
    set({ draft, saved: false });
    clearTimeout(timer);
    timer = setTimeout(() => void ask(), PAUSE_MS);
  }

  return {
    ...empty,

    async loadList() {
      try {
        const done = await commands.agentsList();
        if (done.status === "ok") set({ agents: done.data, error: null });
        else set({ error: done.error });
      } catch (error) {
        set({ error: String(error) });
      }
    },

    async openAgent(name) {
      clearTimeout(timer);
      const mine = ++asked;
      set({ ...empty, agents: get().agents });
      try {
        const done = await commands.agentGet(name);
        // Closed, or another agent opened, while this one was read
        if (mine !== asked) return;
        if (done.status === "error") {
          set({ error: done.error });
          return;
        }
        set({
          open: done.data,
          draft: { frontmatter: done.data.frontmatter, body: done.data.body },
          preview: done.data.resolved,
          previewError: done.data.problem,
        });
      } catch (error) {
        if (mine === asked) set({ error: String(error) });
      }
    },

    setKey(key, value) {
      const { draft } = get();
      if (draft === null) return;
      edited({ ...draft, frontmatter: { ...draft.frontmatter, [key]: value } });
    },

    setBody(body) {
      const { draft } = get();
      if (draft !== null) edited({ ...draft, body });
    },

    setAppend(append) {
      const { draft } = get();
      if (draft === null) return;
      edited({
        ...draft,
        frontmatter: { ...draft.frontmatter, body: append ? "append" : null },
      });
    },

    async save() {
      const { open, draft } = get();
      if (open === null || draft === null) return false;
      try {
        const done = await commands.agentSave(
          open.name,
          clean(draft.frontmatter),
          draft.body,
        );
        if (done.status === "error") {
          // The form stays as it is: the draft is what the person is fixing
          set({ error: done.error });
          return false;
        }
      } catch (error) {
        set({ error: String(error) });
        return false;
      }
      set({ error: null, saved: true });
      void get().loadList();
      return true;
    },

    async create(name, parent) {
      try {
        const done = await commands.agentNew(name, parent);
        if (done.status === "error") {
          set({ error: done.error });
          return false;
        }
      } catch (error) {
        set({ error: String(error) });
        return false;
      }
      set({ error: null });
      await get().loadList();
      return true;
    },

    close() {
      clearTimeout(timer);
      asked++;
      set({ ...empty, agents: get().agents });
    },

    reset() {
      clearTimeout(timer);
      asked++;
      set({ ...empty });
    },
  };
});
