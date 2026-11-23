// The Rekindle mark (plan BR), transcribed from src/assets/brand/*.svg (the
// sources; their NOTICE.md and OFL-Outfit.txt) as static SVG elements, so
// it draws in full colour with no innerHTML (threat model 5b W1). A change
// to the art is made in the .svg and transcribed again.
import { type Component, Match, Switch } from "solid-js";
import type { BrandGlyphName } from "../../ipc/commands/dto";

/** The full flame. */
export const RekindleMark: Component = () => (
  <svg class="skin-node-glyph-svg skin-node-glyph-brand" viewBox="219 178 580 580" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
    <defs>
      <linearGradient id="rk-outer" gradientUnits="userSpaceOnUse" x1="650" y1="250" x2="390" y2="745">
        <stop offset="0" stop-color="#ff59c3" />
        <stop offset="0.26" stop-color="#ff4ec0" />
        <stop offset="0.4" stop-color="#dc4ddc" />
        <stop offset="0.54" stop-color="#9658f0" />
        <stop offset="0.64" stop-color="#3e6cfd" />
        <stop offset="0.8" stop-color="#1270fd" />
        <stop offset="1" stop-color="#0256ee" />
      </linearGradient>
      <linearGradient id="rk-middle" gradientUnits="userSpaceOnUse" x1="430" y1="450" x2="470" y2="740">
        <stop offset="0" stop-color="#b8e4ff" />
        <stop offset="0.55" stop-color="#7fcdff" />
        <stop offset="1" stop-color="#38aaff" />
      </linearGradient>
      <linearGradient id="rk-inner" gradientUnits="userSpaceOnUse" x1="600" y1="320" x2="480" y2="744">
        <stop offset="0" stop-color="#ffbff0" />
        <stop offset="0.28" stop-color="#ffd8f9" />
        <stop offset="0.55" stop-color="#fffdff" />
        <stop offset="0.8" stop-color="#e6f4ff" />
        <stop offset="1" stop-color="#bfe5ff" />
      </linearGradient>
    </defs>
    <path fill="url(#rk-outer)" d="M497 188C530 215 590 270 612 330C622 365 628 400 632 440C640 415 648 395 655 375C690 420 713 490 713 575C713 680 630 748 520 748C400 748 312 680 312 585C312 500 360 440 420 390C470 348 505 290 497 188Z" />
    <path fill="url(#rk-middle)" d="M435 443C400 480 381 530 381 585C381 670 440 744 515 744C560 700 520 560 480 500C460 470 445 455 435 443Z" />
    <path fill="url(#rk-inner)" d="M527 303C560 360 595 430 593 510C592 540 588 560 584 575C605 545 628 510 640 478C655 520 655 590 645 630C630 700 575 744 515 744C470 725 445 690 438 640C428 590 432 520 466 480C495 445 525 380 527 303Z" />
  </svg>
);

/** The two-layer flame for boxes of 16 px or less. */
export const RekindleMarkSmall: Component = () => (
  <svg class="skin-node-glyph-svg skin-node-glyph-brand" viewBox="219 178 580 580" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
    <defs>
      <linearGradient id="rks-outer" gradientUnits="userSpaceOnUse" x1="650" y1="250" x2="390" y2="745">
        <stop offset="0" stop-color="#ff59c3" />
        <stop offset="0.3" stop-color="#ff4ec0" />
        <stop offset="0.55" stop-color="#9658f0" />
        <stop offset="1" stop-color="#0256ee" />
      </linearGradient>
      <linearGradient id="rks-inner" gradientUnits="userSpaceOnUse" x1="560" y1="360" x2="500" y2="740">
        <stop offset="0" stop-color="#ffd8f9" />
        <stop offset="0.5" stop-color="#ffffff" />
        <stop offset="1" stop-color="#cfeaff" />
      </linearGradient>
    </defs>
    <path fill="url(#rks-outer)" d="M497 188C530 215 590 270 612 330C622 365 628 400 632 440C640 415 648 395 655 375C690 420 713 490 713 575C713 680 630 748 520 748C400 748 312 680 312 585C312 500 360 440 420 390C470 348 505 290 497 188Z" />
    <path fill="url(#rks-inner)" d="M528 380C565 440 615 510 615 595C615 665 575 705 515 705C455 705 428 665 428 610C428 530 500 470 528 380Z" />
  </svg>
);

/** The flame and the word "Rekindle" (`rk-word`, filled from the label's colour). */
export const RekindleWordmark: Component = () => (
  <svg class="skin-node-glyph-svg skin-node-glyph-brand" viewBox="0 0 438.2 100" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
    <defs>
      <linearGradient id="rk-outer" gradientUnits="userSpaceOnUse" x1="650" y1="250" x2="390" y2="745">
        <stop offset="0" stop-color="#ff59c3" />
        <stop offset="0.26" stop-color="#ff4ec0" />
        <stop offset="0.4" stop-color="#dc4ddc" />
        <stop offset="0.54" stop-color="#9658f0" />
        <stop offset="0.64" stop-color="#3e6cfd" />
        <stop offset="0.8" stop-color="#1270fd" />
        <stop offset="1" stop-color="#0256ee" />
      </linearGradient>
      <linearGradient id="rk-middle" gradientUnits="userSpaceOnUse" x1="430" y1="450" x2="470" y2="740">
        <stop offset="0" stop-color="#b8e4ff" />
        <stop offset="0.55" stop-color="#7fcdff" />
        <stop offset="1" stop-color="#38aaff" />
      </linearGradient>
      <linearGradient id="rk-inner" gradientUnits="userSpaceOnUse" x1="600" y1="320" x2="480" y2="744">
        <stop offset="0" stop-color="#ffbff0" />
        <stop offset="0.28" stop-color="#ffd8f9" />
        <stop offset="0.55" stop-color="#fffdff" />
        <stop offset="0.8" stop-color="#e6f4ff" />
        <stop offset="1" stop-color="#bfe5ff" />
      </linearGradient>
    </defs>
    <g transform="scale(0.178571) translate(-305 -188)">
      <path fill="url(#rk-outer)" d="M497 188C530 215 590 270 612 330C622 365 628 400 632 440C640 415 648 395 655 375C690 420 713 490 713 575C713 680 630 748 520 748C400 748 312 680 312 585C312 500 360 440 420 390C470 348 505 290 497 188Z" />
      <path fill="url(#rk-middle)" d="M435 443C400 480 381 530 381 585C381 670 440 744 515 744C560 700 520 560 480 500C460 470 445 455 435 443Z" />
      <path fill="url(#rk-inner)" d="M527 303C560 360 595 430 593 510C592 540 588 560 584 575C605 545 628 510 640 478C655 520 655 590 645 630C630 700 575 744 515 744C470 725 445 690 438 640C428 590 432 520 466 480C495 445 525 380 527 303Z" />
    </g>
    <path id="rk-word" fill="#ffffff" d="M103.51 59.18V49.53H118.39Q122.96 49.53 125.47 47.19Q127.98 44.85 127.98 40.74Q127.98 36.98 125.5 34.47Q123.03 31.96 118.46 31.96H103.51V21.83H119.53Q125.74 21.83 130.44 24.26Q135.14 26.69 137.76 30.9Q140.37 35.11 140.37 40.54Q140.37 46.18 137.75 50.35Q135.12 54.53 130.37 56.86Q125.63 59.18 119.31 59.18ZM94.41 86V21.83H106.84V86ZM129.46 86 108.26 58.25 119.69 54.69 144.57 86ZM170.17 86.96Q163.28 86.96 157.86 83.99Q152.44 81.02 149.32 75.79Q146.19 70.55 146.19 63.95Q146.19 57.38 149.26 52.19Q152.33 47 157.61 43.97Q162.89 40.95 169.4 40.95Q175.74 40.95 180.62 43.79Q185.49 46.63 188.27 51.61Q191.06 56.58 191.06 62.95Q191.06 64.11 190.92 65.35Q190.78 66.58 190.46 68.11L153.36 68.18V59.28L184.89 59.21L180.01 62.88Q179.87 58.96 178.63 56.32Q177.39 53.68 175.07 52.29Q172.75 50.9 169.35 50.9Q165.79 50.9 163.17 52.48Q160.54 54.05 159.12 56.91Q157.69 59.78 157.69 63.77Q157.69 67.79 159.21 70.74Q160.73 73.7 163.53 75.29Q166.34 76.87 170.11 76.87Q173.44 76.87 176.2 75.72Q178.96 74.57 180.97 72.28L188 79.36Q184.76 83.12 180.15 85.04Q175.54 86.96 170.17 86.96ZM226.57 86 209.54 63.32 226.48 41.91H240.26L220.36 65.99L220.82 60.21L241.17 86ZM198.45 86V20H210.45V86ZM245.05 86V41.91H257.06V86ZM251.05 34.83Q248.16 34.83 246.24 32.86Q244.32 30.89 244.32 27.99Q244.32 25.09 246.24 23.12Q248.16 21.14 251.05 21.14Q254.04 21.14 255.92 23.12Q257.79 25.09 257.79 27.99Q257.79 30.89 255.92 32.86Q254.04 34.83 251.05 34.83ZM297.31 86V60.74Q297.31 56.79 294.85 54.31Q292.38 51.84 288.54 51.84Q285.93 51.84 283.92 52.95Q281.91 54.07 280.78 56.09Q279.65 58.11 279.65 60.74L274.97 58.32Q274.97 53.18 277.22 49.29Q279.47 45.4 283.43 43.2Q287.39 41 292.41 41Q297.29 41 301.11 43.41Q304.94 45.83 307.13 49.74Q309.32 53.64 309.32 58.07V86ZM267.65 86V41.91H279.65V86ZM337.57 86.91Q331.45 86.91 326.62 83.9Q321.79 80.89 319 75.71Q316.21 70.53 316.21 64.01Q316.21 57.42 318.99 52.24Q321.78 47.07 326.6 44.03Q331.43 41 337.57 41Q342.34 41 346.14 42.94Q349.94 44.88 352.28 48.28Q354.62 51.68 354.78 56.01V71.67Q354.62 76 352.31 79.46Q350.01 82.92 346.17 84.92Q342.34 86.91 337.57 86.91ZM339.62 75.87Q343 75.87 345.49 74.35Q347.98 72.83 349.4 70.15Q350.81 67.47 350.81 63.95Q350.81 60.41 349.4 57.75Q347.99 55.09 345.5 53.57Q343 52.04 339.68 52.04Q336.36 52.04 333.86 53.57Q331.35 55.1 329.9 57.79Q328.44 60.47 328.44 63.95Q328.44 67.45 329.87 70.14Q331.31 72.83 333.85 74.35Q336.39 75.87 339.62 75.87ZM362.08 86H350.19V74.16L352.11 63.41L350.08 52.75V20H362.08ZM372.62 86V20H384.63V86ZM416.28 86.96Q409.39 86.96 403.97 83.99Q398.55 81.02 395.42 75.79Q392.3 70.55 392.3 63.95Q392.3 57.38 395.37 52.19Q398.44 47 403.72 43.97Q409 40.95 415.51 40.95Q421.85 40.95 426.72 43.79Q431.6 46.63 434.38 51.61Q437.16 56.58 437.16 62.95Q437.16 64.11 437.03 65.35Q436.89 66.58 436.57 68.11L399.46 68.18V59.28L431 59.21L426.12 62.88Q425.98 58.96 424.74 56.32Q423.49 53.68 421.18 52.29Q418.86 50.9 415.46 50.9Q411.9 50.9 409.28 52.48Q406.65 54.05 405.23 56.91Q403.8 59.78 403.8 63.77Q403.8 67.79 405.32 70.74Q406.83 73.7 409.64 75.29Q412.45 76.87 416.21 76.87Q419.55 76.87 422.31 75.72Q425.07 74.57 427.08 72.28L434.11 79.36Q430.87 83.12 426.26 85.04Q421.65 86.96 416.28 86.96Z" />
  </svg>
);

/** The served drawing of the mark, centred in its box with its aspect kept. */
export const BrandGlyph: Component<{ glyph: BrandGlyphName }> = (props) => (
  <Switch>
    <Match when={props.glyph === "rekindle"}>
      <RekindleMark />
    </Match>
    <Match when={props.glyph === "rekindleMarkSmall"}>
      <RekindleMarkSmall />
    </Match>
    <Match when={props.glyph === "rekindleWordmark"}>
      <RekindleWordmark />
    </Match>
  </Switch>
);
