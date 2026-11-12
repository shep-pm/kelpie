import { createContext, type ReactNode, useContext } from "react";
import { type ChatStore, useChat } from "./store";

const ChatContext = createContext<ChatStore | null>(null);

/** Who the chat is talking to, which the inline cards name. */
const WhoContext = createContext("PM");

/** Whether the session below it takes a message from the maintainer. */
const ReadOnlyContext = createContext(false);

/**
 * Gives the chat view below it `store`, in place of the PM's.
 *
 * `who` is the name the cards in the turns use, so a worker's tab says who it is
 * asking (`docs/design/system/`, SandboxRequest). `readOnly` is a session the
 * maintainer only watches, which leaves its composer and its Retry out.
 */
export function ChatProvider({
  store,
  who = "PM",
  readOnly = false,
  children,
}: {
  store: ChatStore;
  who?: string;
  readOnly?: boolean;
  children: ReactNode;
}) {
  return (
    <WhoContext.Provider value={who}>
      <ReadOnlyContext.Provider value={readOnly}>
        <ChatContext.Provider value={store}>{children}</ChatContext.Provider>
      </ReadOnlyContext.Provider>
    </WhoContext.Provider>
  );
}

/** The nearest provider's store, or the PM's where there is none. */
export function useChatStore(): ChatStore {
  return useContext(ChatContext) ?? useChat;
}

/** Who the nearest chat is talking to. */
export function useWho(): string {
  return useContext(WhoContext);
}

/** Whether the nearest chat only watches its session. */
export function useReadOnly(): boolean {
  return useContext(ReadOnlyContext);
}
