import { Component, For, Show, createSignal } from "solid-js";
import type { Community } from "../../../../stores/community.store";
import type { ConfirmOptions } from "../../CommunitySettingsModal";
import {
  handleReorderCategories,
  handleCreateCategory,
  handleDeleteCategory,
  handleRenameCategory,
} from "../../../../handlers/community.handlers";
import {
  ICON_SAVE,
  ICON_PENCIL,
  ICON_DELETE,
  ICON_PLUS_BOX,
  ICON_ARROW_UP,
  ICON_ARROW_DOWN,
  ICON_FOLDER,
} from "../../../../icons";

interface CategoryManagerProps {
  community: Community;
  requestConfirm: (opts: ConfirmOptions) => void;
}

const CategoryManager: Component<CategoryManagerProps> = (props) => {
  const [renamingCategoryId, setRenamingCategoryId] = createSignal<string | null>(null);
  const [categoryRenameValue, setCategoryRenameValue] = createSignal("");
  const [showNewCategory, setShowNewCategory] = createSignal(false);
  const [newCategoryName, setNewCategoryName] = createSignal("");

  const sortedCategories = () =>
    [...props.community.categories].sort((a, b) => a.sortOrder - b.sortOrder);

  function startCategoryRename(cat: { id: string; name: string }): void {
    setRenamingCategoryId(cat.id);
    setCategoryRenameValue(cat.name);
  }

  async function submitCategoryRename(categoryId: string): Promise<void> {
    const val = categoryRenameValue().trim();
    if (val) {
      await handleRenameCategory(props.community.id, categoryId, val);
    }
    setRenamingCategoryId(null);
  }

  function confirmDeleteCategory(cat: { id: string; name: string }): void {
    props.requestConfirm({
      title: "Delete Category",
      message: `Delete category "${cat.name}"? Channels will become uncategorized.`,
      confirmLabel: "Delete",
      action: () => handleDeleteCategory(props.community.id, cat.id),
    });
  }

  async function handleCreateCat(): Promise<void> {
    const n = newCategoryName().trim();
    if (!n) return;
    await handleCreateCategory(props.community.id, n);
    setNewCategoryName("");
    setShowNewCategory(false);
  }

  function moveCategoryUp(index: number): void {
    const sorted = sortedCategories();
    if (index <= 0) return;
    const ids = sorted.map((c) => c.id);
    [ids[index - 1], ids[index]] = [ids[index], ids[index - 1]];
    handleReorderCategories(props.community.id, ids);
  }

  function moveCategoryDown(index: number): void {
    const sorted = sortedCategories();
    if (index >= sorted.length - 1) return;
    const ids = sorted.map((c) => c.id);
    [ids[index], ids[index + 1]] = [ids[index + 1], ids[index]];
    handleReorderCategories(props.community.id, ids);
  }

  return (
    <div class="settings-subsection">
      <h4 class="settings-subsection-title">Categories</h4>
      <For each={sortedCategories()}>
        {(cat, index) => (
          <div class="channel-manage-row">
            <span class="nf-icon channel-manage-icon">{ICON_FOLDER}</span>
            <Show when={renamingCategoryId() === cat.id} fallback={
              <span class="channel-manage-name">{cat.name}</span>
            }>
              <input
                class="form-input channel-rename-input"
                type="text"
                value={categoryRenameValue()}
                onInput={(e) => setCategoryRenameValue(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") submitCategoryRename(cat.id);
                  if (e.key === "Escape") setRenamingCategoryId(null);
                }}
              />
            </Show>
            <Show when={renamingCategoryId() !== cat.id}>
              <button
                class="form-btn-secondary channel-manage-btn"
                onClick={() => startCategoryRename(cat)}
                title="Rename"
              >
                <span class="nf-icon">{ICON_PENCIL}</span>
              </button>
            </Show>
            <Show when={renamingCategoryId() === cat.id}>
              <button
                class="form-btn-primary channel-manage-btn"
                onClick={() => submitCategoryRename(cat.id)}
                title="Save"
              >
                <span class="nf-icon">{ICON_SAVE}</span>
              </button>
            </Show>
            <button
              class="form-btn-danger channel-manage-btn"
              onClick={() => confirmDeleteCategory(cat)}
              title="Delete"
            >
              <span class="nf-icon">{ICON_DELETE}</span>
            </button>
            <button
              class="form-btn-secondary channel-manage-btn"
              onClick={() => moveCategoryUp(index())}
              disabled={index() === 0}
              title="Move Up"
            >
              <span class="nf-icon">{ICON_ARROW_UP}</span>
            </button>
            <button
              class="form-btn-secondary channel-manage-btn"
              onClick={() => moveCategoryDown(index())}
              disabled={index() === sortedCategories().length - 1}
              title="Move Down"
            >
              <span class="nf-icon">{ICON_ARROW_DOWN}</span>
            </button>
          </div>
        )}
      </For>
      <Show when={showNewCategory()} fallback={
        <button
          class="form-btn-secondary"
          onClick={() => setShowNewCategory(true)}
        >
          <span class="nf-icon">{ICON_PLUS_BOX}</span> Create Category
        </button>
      }>
        <div class="channel-create-inline">
          <input
            class="form-input"
            type="text"
            placeholder="Category name..."
            value={newCategoryName()}
            onInput={(e) => setNewCategoryName(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleCreateCat();
              if (e.key === "Escape") setShowNewCategory(false);
            }}
          />
          <button
            class="form-btn-primary"
            onClick={handleCreateCat}
            disabled={!newCategoryName().trim()}
          >
            Create
          </button>
          <button
            class="form-btn-secondary"
            onClick={() => setShowNewCategory(false)}
          >
            Cancel
          </button>
        </div>
      </Show>
    </div>
  );
};

export default CategoryManager;
