// 附件域 Tauri 命令封装（与 commands/attachment.rs 对应）。
import { invoke } from '@tauri-apps/api/core';
import type { StoredAttachment } from '../store/types';

/** 把本地文件注册为聊天附件（拷贝到附件存储并抽取文本），返回附件元数据。 */
export const storeChatAttachment = (path: string) =>
  invoke<StoredAttachment>('store_chat_attachment', { path });

/** 丢弃一个尚未关联消息的附件（撤回未发送的上传）。 */
export const discardChatAttachment = (attachmentId: string) =>
  invoke<void>('discard_chat_attachment', { attachmentId });

/** 保存二进制内容到文件（导出/下载）。 */
export const saveBinaryFile = (path: string, bytes: number[]) =>
  invoke<void>('save_binary_file', { path, bytes });

/** 把一张生成/存储的图片复制到目标路径（下载）。 */
export const copyStoredImage = (src: string, dest: string) =>
  invoke<void>('copy_stored_image', { src, dest });
