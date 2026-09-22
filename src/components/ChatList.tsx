import { useEffect, useState } from "react";
import { chatsList, chatsRemove, type ChatSummary } from "../api";

/**
 * Разговоры в боковом меню: новые сверху.
 * `refresh` меняется, когда разговор сохранился, — это повод перечитать список.
 */
export default function ChatList({
  current,
  refresh,
  onPick,
  onNew,
  onRemoved,
}: {
  current: string | null;
  refresh: number;
  onPick: (id: string) => void;
  onNew: () => void;
  onRemoved: (id: string) => void;
}) {
  const [items, setItems] = useState<ChatSummary[]>([]);

  useEffect(() => {
    chatsList().then(setItems);
  }, [refresh]);

  const remove = async (id: string) => {
    await chatsRemove(id);
    setItems((prev) => prev.filter((c) => c.id !== id));
    onRemoved(id);
  };

  return (
    <div className="chats">
      <button className="tab new" onClick={onNew}>
        + Новый разговор
      </button>
      {items.map((c) => (
        <div key={c.id} className={c.id === current ? "chat-row active" : "chat-row"}>
          <button className="tab" title={c.title} onClick={() => onPick(c.id)}>
            {c.title}
          </button>
          <button className="link forget" title="Удалить разговор" onClick={() => remove(c.id)}>
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}
