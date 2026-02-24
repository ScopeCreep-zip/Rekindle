import { Component, For, Show, createSignal, createMemo, onMount, onCleanup } from "solid-js";

const EMOJI_CATEGORIES: { name: string; icon: string; emojis: string[] }[] = [
  {
    name: "Gaming",
    icon: "\uD83C\uDFAE",
    emojis: [
      "\uD83D\uDC4D", "\uD83D\uDC4E", "\u2764\uFE0F", "\uD83D\uDD25", "\uD83D\uDE02",
      "\uD83D\uDE2E", "\uD83D\uDE22", "\uD83D\uDE21", "\uD83C\uDFAE", "\uD83C\uDFC6",
      "\u2694\uFE0F", "\uD83D\uDEE1\uFE0F", "\uD83D\uDC80", "\uD83D\uDC7B", "\uD83C\uDFAF",
      "\uD83D\uDC8E", "\u2B50", "\uD83D\uDE80", "\uD83D\uDCAA", "\uD83D\uDC40",
      "\uD83E\uDD14", "\uD83D\uDC4F", "\u2705", "\u274C", "\uD83D\uDCAF",
    ],
  },
  {
    name: "Faces",
    icon: "\uD83D\uDE00",
    emojis: [
      "\uD83D\uDE00", "\uD83D\uDE03", "\uD83D\uDE04", "\uD83D\uDE01", "\uD83D\uDE06",
      "\uD83D\uDE05", "\uD83E\uDD23", "\uD83D\uDE0A", "\uD83D\uDE07", "\uD83D\uDE0D",
      "\uD83E\uDD29", "\uD83D\uDE18", "\uD83D\uDE1C", "\uD83E\uDD2A", "\uD83E\uDD28",
      "\uD83E\uDDD0", "\uD83E\uDD13", "\uD83D\uDE0E", "\uD83E\uDD78", "\uD83E\uDD25",
      "\uD83D\uDE2C", "\uD83D\uDE10", "\uD83D\uDE11", "\uD83D\uDE36", "\uD83E\uDD2F",
      "\uD83D\uDE33", "\uD83E\uDD7A", "\uD83D\uDE25", "\uD83D\uDE2D", "\uD83D\uDE31",
      "\uD83D\uDE28", "\uD83D\uDE30", "\uD83E\uDD2E", "\uD83D\uDE34", "\uD83D\uDE24",
      "\uD83D\uDE12", "\uD83D\uDE2A", "\uD83E\uDD74", "\uD83D\uDE37", "\uD83E\uDD15",
    ],
  },
  {
    name: "Gestures",
    icon: "\uD83D\uDC4B",
    emojis: [
      "\uD83D\uDC4B", "\uD83E\uDD1A", "\uD83D\uDD90\uFE0F", "\u270B", "\uD83D\uDD96",
      "\uD83E\uDD1E", "\uD83E\uDD1F", "\uD83E\uDD18", "\uD83E\uDD19", "\uD83D\uDC48",
      "\uD83D\uDC49", "\uD83D\uDC46", "\uD83D\uDC47", "\u261D\uFE0F", "\uD83D\uDC4C",
      "\uD83E\uDD0C", "\uD83E\uDD0F", "\u270C\uFE0F", "\uD83E\uDD1E", "\uD83E\uDD19",
    ],
  },
  {
    name: "Hearts",
    icon: "\u2764\uFE0F",
    emojis: [
      "\u2764\uFE0F", "\uD83E\uDE77", "\uD83E\uDDE1", "\uD83D\uDC9B", "\uD83D\uDC9A",
      "\uD83D\uDC99", "\uD83D\uDC9C", "\uD83E\uDD0E", "\uD83D\uDDA4", "\uD83E\uDD0D",
      "\uD83D\uDC94", "\u2763\uFE0F", "\uD83D\uDC95", "\uD83D\uDC96", "\uD83D\uDC97",
      "\uD83D\uDC98", "\uD83D\uDC9D", "\uD83D\uDC9F", "\u2764\uFE0F\u200D\uD83D\uDD25",
      "\u2764\uFE0F\u200D\uD83E\uDE79",
    ],
  },
  {
    name: "Objects",
    icon: "\uD83D\uDCBB",
    emojis: [
      "\uD83D\uDCBB", "\uD83D\uDCF1", "\uD83C\uDFB5", "\uD83C\uDFB6", "\uD83D\uDD14",
      "\uD83D\uDCA1", "\uD83D\uDCE3", "\uD83D\uDCDD", "\uD83D\uDD12", "\uD83D\uDD13",
      "\uD83D\uDC8D", "\uD83C\uDF89", "\uD83C\uDF81", "\uD83C\uDF8A", "\uD83C\uDFA8",
      "\uD83D\uDE97", "\u2708\uFE0F", "\uD83D\uDD28", "\uD83D\uDEE0\uFE0F", "\u2699\uFE0F",
    ],
  },
  {
    name: "Nature",
    icon: "\uD83C\uDF1F",
    emojis: [
      "\uD83C\uDF1F", "\u2600\uFE0F", "\uD83C\uDF19", "\u26A1", "\uD83C\uDF08",
      "\uD83C\uDF0A", "\uD83C\uDF3F", "\uD83C\uDF38", "\uD83C\uDF31", "\uD83C\uDF4E",
      "\uD83D\uDC36", "\uD83D\uDC31", "\uD83E\uDD8A", "\uD83E\uDD89", "\uD83E\uDD87",
    ],
  },
];

const EMOJI_SEARCH_NAMES: Record<string, string[]> = {
  "\uD83D\uDC4D": ["thumbs up", "like", "yes", "ok"],
  "\uD83D\uDC4E": ["thumbs down", "dislike", "no"],
  "\u2764\uFE0F": ["heart", "love", "red heart"],
  "\uD83D\uDD25": ["fire", "hot", "lit"],
  "\uD83D\uDE02": ["laugh", "lol", "cry laugh", "tears"],
  "\uD83D\uDE2E": ["surprised", "wow", "shocked"],
  "\uD83D\uDE22": ["sad", "cry"],
  "\uD83D\uDE21": ["angry", "mad"],
  "\uD83C\uDFAE": ["game", "controller", "gaming"],
  "\uD83C\uDFC6": ["trophy", "winner", "champion"],
  "\u2694\uFE0F": ["swords", "battle", "fight"],
  "\uD83D\uDEE1\uFE0F": ["shield", "defend", "protect"],
  "\uD83D\uDC80": ["skull", "dead", "death"],
  "\uD83D\uDC7B": ["ghost", "spooky"],
  "\uD83C\uDFAF": ["target", "bullseye", "aim"],
  "\uD83D\uDC8E": ["gem", "diamond"],
  "\u2B50": ["star", "favorite"],
  "\uD83D\uDE80": ["rocket", "launch", "fast"],
  "\uD83D\uDCAA": ["muscle", "strong", "flex"],
  "\uD83D\uDC40": ["eyes", "look", "see"],
  "\uD83E\uDD14": ["thinking", "hmm"],
  "\uD83D\uDC4F": ["clap", "applause"],
  "\u2705": ["check", "yes", "done"],
  "\u274C": ["x", "no", "wrong"],
  "\uD83D\uDCAF": ["100", "perfect"],
  "\uD83D\uDE00": ["grin", "happy"],
  "\uD83D\uDE0D": ["heart eyes", "love"],
  "\uD83D\uDE0E": ["cool", "sunglasses"],
  "\uD83D\uDE2D": ["sob", "crying"],
  "\uD83E\uDD23": ["rofl", "rolling"],
  "\uD83D\uDC4B": ["wave", "hi", "hello"],
  "\uD83D\uDC94": ["broken heart"],
  "\uD83D\uDCBB": ["computer", "laptop"],
  "\uD83C\uDF89": ["party", "celebrate"],
  "\uD83C\uDF1F": ["star", "sparkle"],
};

interface EmojiPickerProps {
  onSelect: (emoji: string) => void;
  onClose: () => void;
}

const EmojiPicker: Component<EmojiPickerProps> = (props) => {
  let ref: HTMLDivElement | undefined;
  let searchRef: HTMLInputElement | undefined;
  const [searchQuery, setSearchQuery] = createSignal("");
  const [activeCategory, setActiveCategory] = createSignal(0);

  function handleClickOutside(e: MouseEvent): void {
    if (ref && !ref.contains(e.target as Node)) {
      props.onClose();
    }
  }

  onMount(() => {
    document.addEventListener("mousedown", handleClickOutside);
    requestAnimationFrame(() => searchRef?.focus());
  });

  onCleanup(() => {
    document.removeEventListener("mousedown", handleClickOutside);
  });

  const filteredCategories = createMemo(() => {
    const q = searchQuery().toLowerCase().trim();
    if (!q) return EMOJI_CATEGORIES;

    const allMatching: string[] = [];
    for (const cat of EMOJI_CATEGORIES) {
      for (const emoji of cat.emojis) {
        const names = EMOJI_SEARCH_NAMES[emoji] ?? [];
        if (names.some((n) => n.includes(q)) || emoji.includes(q)) {
          allMatching.push(emoji);
        }
      }
    }
    if (allMatching.length === 0) return [];
    return [{ name: "Results", icon: "\uD83D\uDD0D", emojis: allMatching }];
  });

  return (
    <div class="emoji-picker" ref={ref}>
      <input
        ref={searchRef}
        class="emoji-picker-search"
        type="text"
        placeholder="Search emoji..."
        value={searchQuery()}
        onInput={(e) => setSearchQuery(e.currentTarget.value)}
      />
      <Show when={!searchQuery()}>
        <div class="emoji-picker-tabs">
          <For each={EMOJI_CATEGORIES}>
            {(cat, idx) => (
              <button
                class={`emoji-picker-tab ${activeCategory() === idx() ? "emoji-picker-tab-active" : ""}`}
                onClick={() => {
                  setActiveCategory(idx());
                  const section = ref?.querySelector(`[data-category="${idx()}"]`);
                  section?.scrollIntoView({ behavior: "smooth" });
                }}
              >
                {cat.icon}
              </button>
            )}
          </For>
        </div>
      </Show>
      <div class="emoji-picker-scroll">
        <For each={filteredCategories()}>
          {(cat, idx) => (
            <div data-category={idx()}>
              <div class="emoji-picker-section-label">{cat.name}</div>
              <div class="emoji-picker-grid">
                <For each={cat.emojis}>
                  {(emoji) => (
                    <button
                      class="emoji-picker-item"
                      onClick={() => {
                        props.onSelect(emoji);
                        props.onClose();
                      }}
                    >
                      {emoji}
                    </button>
                  )}
                </For>
              </div>
            </div>
          )}
        </For>
        <Show when={filteredCategories().length === 0}>
          <div class="emoji-picker-empty">No emoji found</div>
        </Show>
      </div>
    </div>
  );
};

export default EmojiPicker;
