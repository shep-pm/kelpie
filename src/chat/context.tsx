import { createContext, type ReactNode, useContext } from "react";
import { type ChatStore, useChat } from "./store";

const ChatContext = createContext<ChatStore | null>(null);

/** Gives the chat view below it `store`, in place of the PM's. */
export function ChatProvider({
  store,
  children,
}: {
  store: ChatStore;
  children: ReactNode;
}) {
  return <ChatContext.Provider value={store}>{children}</ChatContext.Provider>;
}

/** The nearest provider's store, or the PM's where there is none. */
export function useChatStore(): ChatStore {
  return useContext(ChatContext) ?? useChat;
}
