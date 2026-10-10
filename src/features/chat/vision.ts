// Auto-vision helpers: deciding whether the selected model can see images
// and which activated model acts as the vision agent's eyes.
// Kept out of ChatPage.tsx for the features file-size gate.

import { getCachedCatalog } from '../../core/utils/models';
import { allAvailableModels, modelKey, profileModelOverrides } from '../../core/store/store';

/**
 * Whether the activated model's catalog metadata declares vision capability.
 * Models missing from the catalog are conservatively treated as non-vision
 * (the auto-vision path then routes images through the vision agent).
 */
export function modelSupportsVision(m: { model_id: string }): boolean {
  try {
    const cat = getCachedCatalog();
    const meta = cat?.models?.find((x: any) => x.id === m.model_id) as any;
    return !!meta?.capabilities?.vision;
  } catch {
    return false;
  }
}

/**
 * Pick the model that acts as the vision agent's eyes when the selected model
 * cannot see images: the explicit per-profile override for "vision" first
 * (Settings -> Subagent models), then any other activated model whose catalog
 * metadata declares vision. Returns null when no vision model is available.
 */
export function resolveVisionModel(current: { model_id: string }) {
  const all = allAvailableModels();
  const overrideKey = profileModelOverrides()['vision'];
  if (overrideKey) {
    const found = all.find((x) => modelKey(x) === overrideKey);
    if (found) return found;
  }
  const cat = getCachedCatalog();
  if (!cat) return null;
  return (
    all.find(
      (x) =>
        x.model_id !== current.model_id &&
        !!(cat.models?.find((mm: any) => mm.id === x.model_id) as any)?.capabilities?.vision,
    ) ?? null
  );
}
