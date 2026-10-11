/**
 * Message-level selection mode (阶段4 第2项拆分切片：ChatPage <900 目标).
 *
 * Verbatim extraction of the selection family from ChatPage: the four selection
 * signals (mode flag, selected-id set, share-modal flag, share-source topic) plus
 * the handlers (enter / toggle / select-all / cancel / confirm) and the
 * share-source topic resolution. The signals were component-scoped because both
 * ChatInterface (checkbox rows) and the TopicSidebar context menu drive them;
 * the hook owns them and returns accessors + setters so the JSX wiring is
 * unchanged.
 */
import { createSignal, type Accessor } from 'solid-js';
import { setCurrentTopicId } from '../../../core/store/store';
import type { Topic } from '../../../core/store/store';

/** Deps the ChatPage component provides (its computed accessors). */
export interface MessageSelectionDeps {
  currentAssistant: () => { id: string; topics: Topic[] } | undefined;
  activeTopic: () => Topic | null;
}

export function useMessageSelection(deps: MessageSelectionDeps) {
  const [isSelectingMessages, setIsSelectingMessages] = createSignal(false);
  const [selectedMessageIds, setSelectedMessageIds] = createSignal<Set<string>>(new Set());
  const [showShareModal, setShowShareModal] = createSignal(false);
  const [activeShareTopicId, setActiveShareTopicId] = createSignal<string | null>(null);

  /** The topic the share modal exports (explicit selection source, else the active topic). */
  const shareTopic = (): Topic | null => {
    const asst = deps.currentAssistant();
    if (!asst) return null;
    const sid = activeShareTopicId();
    if (sid) return asst.topics.find((t: Topic) => t.id === sid) || null;
    return deps.activeTopic();
  };

  /** Enter selection mode */
  const enterSelectionMode = (topicId?: string | null) => {
    if (topicId) {
      setActiveShareTopicId(topicId);
      setCurrentTopicId(topicId);
    }
    setSelectedMessageIds(new Set<string>());
    setIsSelectingMessages(true);
  };

  /** Toggle the selected state of one message */
  const handleToggleMessage = (msgId: string) => {
    setSelectedMessageIds((prev) => {
      const next = new Set<string>(prev);
      if (next.has(msgId)) next.delete(msgId);
      else next.add(msgId);
      return next;
    });
  };

  /** Select all exportable messages in the current topic (excluding tool messages) */
  const handleSelectAll = () => {
    const topic = deps.activeTopic();
    if (!topic) return;
    const ids = new Set<string>();
    for (const msg of topic.history) {
      if (msg.role !== 'tool' && msg.id) ids.add(msg.id);
    }
    setSelectedMessageIds(ids);
  };

  /** Cancel the selection and exit selection mode */
  const handleCancelSelection = () => {
    setIsSelectingMessages(false);
    setSelectedMessageIds(new Set<string>());
    setActiveShareTopicId(null);
  };

  /** Confirm the selection and open the share modal */
  const handleConfirmSelection = () => {
    setIsSelectingMessages(false);
    setShowShareModal(true);
  };

  /** Close the share modal and reset the selection state (ShareModal onClose). */
  const closeShareModal = () => {
    setShowShareModal(false);
    setActiveShareTopicId(null);
    setSelectedMessageIds(new Set<string>());
  };

  return {
    isSelectingMessages,
    selectedMessageIds,
    showShareModal,
    setShowShareModal,
    activeShareTopicId,
    shareTopic,
    enterSelectionMode,
    handleToggleMessage,
    handleSelectAll,
    handleCancelSelection,
    handleConfirmSelection,
    closeShareModal,
  };
}

export type MessageSelection = ReturnType<typeof useMessageSelection>;
export type { Accessor };
