// The swallow from the app icon (design/make-icon.swift), on the same 24-unit grid.
export function Logo({ size = 20 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" className="logo">
      <path
        fill="currentColor"
        d="M22 4.2C20.4 3.4 18.6 3.5 17.2 4.6C15.6 5.8 14.2 7.2 12.6 8.4C9 6.4 4.8 5.4 1.5 6.2C4.6 8.6 7.6 10.4 10.4 11.6C7.6 14 4.8 16.6 2.2 18.8C5 17.6 7.6 16.2 9.6 15C8.2 17.2 6.8 19.4 5.6 21.4C9.6 18.6 13.2 15 15.6 11.6C17.2 9.6 18 7.6 19.2 6.6C20.2 5.8 21.2 5 22 4.2Z"
      />
      <path fill="var(--hot)" d="M20.7 5.3C19.9 5.4 19.1 5.5 18.6 5.9C18.6 6.2 18.9 6.5 19.2 6.6C19.8 6.1 20.3 5.6 20.7 5.3Z" />
    </svg>
  );
}
