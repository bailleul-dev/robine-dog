import type { SVGProps } from "react";

type P = SVGProps<SVGSVGElement>;
const base = { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.8, strokeLinecap: "round", strokeLinejoin: "round" } as const;

export const Logo = (p: P) => (
  <svg viewBox="0 0 32 32" {...p}>
    <rect width="32" height="32" rx="7" fill="#632ca6" />
    <path d="M9 22c0-5.2 3.1-9.2 7-9.2s7 4 7 9.2" fill="none" stroke="#fff" strokeWidth="2.4" strokeLinecap="round" />
    <circle cx="13.2" cy="17.6" r="1.5" fill="#fff" />
    <circle cx="18.8" cy="17.6" r="1.5" fill="#fff" />
    <path d="M7.5 13.5 10.6 8.6l2.3 4.6M24.5 13.5l-3.1-4.9-2.3 4.6" fill="#fff" />
  </svg>
);

export const LogsIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M4 5h16M4 10h16M4 15h10M4 20h7" />
  </svg>
);

export const ApmIcon = (p: P) => (
  <svg {...base} {...p}>
    <rect x="3" y="4" width="8" height="4" rx="1" />
    <rect x="9" y="10" width="12" height="4" rx="1" />
    <rect x="6" y="16" width="7" height="4" rx="1" />
  </svg>
);

export const SearchIcon = (p: P) => (
  <svg {...base} {...p}>
    <circle cx="11" cy="11" r="6.5" />
    <path d="m20 20-4.2-4.2" />
  </svg>
);

export const ClockIcon = (p: P) => (
  <svg {...base} {...p}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 7.5V12l3 2" />
  </svg>
);

export const PlayIcon = (p: P) => (
  <svg viewBox="0 0 24 24" fill="currentColor" {...p}>
    <path d="M8 5.5v13l10.5-6.5z" />
  </svg>
);

export const PauseIcon = (p: P) => (
  <svg viewBox="0 0 24 24" fill="currentColor" {...p}>
    <rect x="6.5" y="5" width="4" height="14" rx="1" />
    <rect x="13.5" y="5" width="4" height="14" rx="1" />
  </svg>
);

export const ChevronDown = (p: P) => (
  <svg {...base} {...p}>
    <path d="m6 9 6 6 6-6" />
  </svg>
);

export const ChevronRight = (p: P) => (
  <svg {...base} {...p}>
    <path d="m9 6 6 6-6 6" />
  </svg>
);

export const ChevronUp = (p: P) => (
  <svg {...base} {...p}>
    <path d="m6 15 6-6 6 6" />
  </svg>
);

export const CloseIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M6 6l12 12M18 6 6 18" />
  </svg>
);

export const CopyIcon = (p: P) => (
  <svg {...base} {...p}>
    <rect x="8" y="8" width="12" height="12" rx="2" />
    <path d="M16 8V5.5A1.5 1.5 0 0 0 14.5 4h-9A1.5 1.5 0 0 0 4 5.5v9A1.5 1.5 0 0 0 5.5 16H8" />
  </svg>
);

export const FilterIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M4 5h16l-6 7.5V19l-4-2v-4.5z" />
  </svg>
);

export const ExcludeIcon = (p: P) => (
  <svg {...base} {...p}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M8 12h8" />
  </svg>
);

export const PlusIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M12 5v14M5 12h14" />
  </svg>
);

export const TraceIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M4 6h10M7 12h10M10 18h10" />
  </svg>
);

export const RefreshIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6" />
  </svg>
);

export const SignOutIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M15 4h3a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-3M10 17l5-5-5-5M15 12H4" />
  </svg>
);

export const LiveIcon = (p: P) => (
  <svg {...base} {...p}>
    <circle cx="12" cy="12" r="2.5" fill="currentColor" />
    <path d="M7.8 7.8a6 6 0 0 0 0 8.4M16.2 7.8a6 6 0 0 1 0 8.4M5 5a10 10 0 0 0 0 14M19 5a10 10 0 0 1 0 14" />
  </svg>
);

export const ServiceIcon = (p: P) => (
  <svg {...base} {...p}>
    <rect x="4" y="4" width="16" height="6" rx="1.5" />
    <rect x="4" y="14" width="16" height="6" rx="1.5" />
    <path d="M8 7h.01M8 17h.01" />
  </svg>
);

export const ExternalIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M14 4h6v6M20 4l-9 9M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4" />
  </svg>
);

export const TrashIcon = (p: P) => (
  <svg {...base} {...p}>
    <path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3" />
  </svg>
);
