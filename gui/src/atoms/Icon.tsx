/**
 * Icon set ported verbatim from `gui/assets/atoms.jsx`. Inline SVG —
 * no icon-font / no svg-loader plugin so the bundle stays small.
 */

import type { JSX } from "react";

export type IconName =
  | "timeline"
  | "memory"
  | "decision"
  | "law"
  | "analysis"
  | "tools"
  | "graph"
  | "search"
  | "filter"
  | "settings"
  | "bell"
  | "book"
  | "menu"
  | "close"
  | "play"
  | "pause"
  | "moon"
  | "sun"
  | "chevron-right"
  | "chevron-down"
  | "arrow-up"
  | "arrow-right"
  | "external"
  | "shield"
  | "spark"
  | "tag"
  | "git"
  | "block"
  | "alert";

type IconProps = { name: IconName; size?: number; stroke?: number };

export function Icon({ name, size = 16, stroke = 1.6 }: IconProps): JSX.Element | null {
  const props = {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: stroke,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
  };
  switch (name) {
    case "timeline":
      return (<svg {...props}><path d="M3 6h18M3 12h12M3 18h18"/><circle cx="20" cy="12" r="1.6" fill="currentColor"/></svg>);
    case "memory":
      return (<svg {...props}><rect x="3" y="6" width="18" height="12" rx="2"/><path d="M7 10v4M11 10v4M15 10v4M19 10v4"/></svg>);
    case "decision":
      return (<svg {...props}><path d="M12 3l8 4v6c0 4.5-3.5 7.5-8 8-4.5-.5-8-3.5-8-8V7l8-4z"/><path d="M9 12l2 2 4-4"/></svg>);
    case "law":
      return (<svg {...props}><path d="M3 6h18M6 6l-2 6h6l-2-6M18 6l-2 6h6l-2-6M12 3v18M5 21h14"/></svg>);
    case "analysis":
      return (<svg {...props}><circle cx="11" cy="11" r="6"/><path d="M16 16l5 5"/><path d="M8 11h6M11 8v6"/></svg>);
    case "tools":
      return (<svg {...props}><path d="M3 18l8-8M11 10l3 3 5-5-3-3-5 5zM3 14l4 4M5 16h2"/></svg>);
    case "graph":
      return (<svg {...props}><circle cx="6" cy="6" r="2.4"/><circle cx="18" cy="6" r="2.4"/><circle cx="6" cy="18" r="2.4"/><circle cx="18" cy="18" r="2.4"/><circle cx="12" cy="12" r="2.4"/><path d="M8 7l3 4M16 7l-3 4M8 17l3-4M16 17l-3-4"/></svg>);
    case "search":
      return (<svg {...props}><circle cx="11" cy="11" r="7"/><path d="M16 16l5 5"/></svg>);
    case "filter":
      return (<svg {...props}><path d="M3 5h18M6 12h12M10 19h4"/></svg>);
    case "settings":
      return (<svg {...props}><circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.4.8a7 7 0 0 0-2-1.2L14 3h-4l-.5 2.5a7 7 0 0 0-2 1.2L5 6 3 9.4l2 1.4A7 7 0 0 0 5 12a7 7 0 0 0 .1 1.2L3 14.6 5 18l2.4-.8a7 7 0 0 0 2 1.2L10 21h4l.5-2.5a7 7 0 0 0 2-1.2L19 18l2-3.4-2-1.4z"/></svg>);
    case "bell":
      return (<svg {...props}><path d="M6 16V11a6 6 0 1 1 12 0v5l1.5 2h-15L6 16zM10 20a2 2 0 0 0 4 0"/></svg>);
    case "book":
      return (<svg {...props}><path d="M4 4h12a3 3 0 0 1 3 3v13H7a3 3 0 0 1-3-3V4z"/><path d="M4 17a3 3 0 0 1 3-3h12"/></svg>);
    case "menu":
      return (<svg {...props}><path d="M4 7h16M4 12h16M4 17h16"/></svg>);
    case "close":
      return (<svg {...props}><path d="M5 5l14 14M19 5L5 19"/></svg>);
    case "play":
      return (<svg {...props}><path d="M6 4l14 8-14 8z" fill="currentColor"/></svg>);
    case "pause":
      return (<svg {...props}><rect x="6" y="4" width="4" height="16" rx="1" fill="currentColor"/><rect x="14" y="4" width="4" height="16" rx="1" fill="currentColor"/></svg>);
    case "moon":
      return (<svg {...props}><path d="M20 14a8 8 0 0 1-10-10 8 8 0 1 0 10 10z"/></svg>);
    case "sun":
      return (<svg {...props}><circle cx="12" cy="12" r="4"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M5 19l2-2M17 7l2-2"/></svg>);
    case "chevron-right":
      return (<svg {...props}><path d="M9 6l6 6-6 6"/></svg>);
    case "chevron-down":
      return (<svg {...props}><path d="M6 9l6 6 6-6"/></svg>);
    case "arrow-up":
      return (<svg {...props}><path d="M12 19V5M6 11l6-6 6 6"/></svg>);
    case "arrow-right":
      return (<svg {...props}><path d="M5 12h14M13 5l7 7-7 7"/></svg>);
    case "external":
      return (<svg {...props}><path d="M14 4h6v6M20 4l-9 9M19 13v6H5V5h6"/></svg>);
    case "shield":
      return (<svg {...props}><path d="M12 3l8 3v6c0 4.5-3.5 7.5-8 9-4.5-1.5-8-4.5-8-9V6l8-3z"/></svg>);
    case "spark":
      return (<svg {...props}><path d="M12 2l2.4 6.6L21 11l-6.6 2.4L12 20l-2.4-6.6L3 11l6.6-2.4z" fill="currentColor" stroke="none"/></svg>);
    case "tag":
      return (<svg {...props}><path d="M3 12V3h9l9 9-9 9-9-9z"/><circle cx="8" cy="8" r="1.5" fill="currentColor"/></svg>);
    case "git":
      return (<svg {...props}><circle cx="6" cy="6" r="2.4"/><circle cx="18" cy="6" r="2.4"/><circle cx="6" cy="18" r="2.4"/><path d="M6 8.4v7.2M8.4 6c4 0 6 2 6 6v3.6"/></svg>);
    case "block":
      return (<svg {...props}><circle cx="12" cy="12" r="9"/><path d="M5.5 5.5l13 13"/></svg>);
    case "alert":
      return (<svg {...props}><path d="M12 3l10 17H2L12 3z"/><path d="M12 10v5M12 18v.5" strokeWidth="2"/></svg>);
    default:
      return null;
  }
}
