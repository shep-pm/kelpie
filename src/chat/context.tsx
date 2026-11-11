import { createContext, type ReactNode, useContext } from "react";
import { type ChatStore, useChat } from "./store";

const ChatContext = createContext<ChatStore | null>(null);

/** Who the chat is talking to, which the inline cards name. */
const WhoContext = createContext("PM");

/**
 * Gives the chat view below it `store`, in place of the PM's.
 *
 * `who` is the name the cards in the turns use, so a worker's tab says who it is
 * asking (`docs/design/system/`, SandboxRequest).
 */
export function ChatProvider({
  store,
  who = "PM",
  children,
}: {
  store: ChatStore;
  who?: string;
  children: ReactNode;
}) {
  return (
    <WhoContext.Provider value={who}>
      <ChatContext.Provider value={store}>{children}</ChatContext.Provider>
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
