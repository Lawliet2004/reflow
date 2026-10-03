import { useLayoutEffect, useRef, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import "./TranscriptMenu.css";

interface TranscriptMenuProps {
  id: string;
  trigger: HTMLButtonElement;
  focusLast?: boolean;
  onClose: (restoreFocus?: boolean) => void;
  children: ReactNode;
}

function isPageTabStop(control: HTMLElement): boolean {
  if (
    control.tabIndex < 0 ||
    control.matches(':disabled, input[type="hidden"]') ||
    control.closest('[hidden], [inert], [aria-hidden="true"]')
  )
    return false;

  for (let ancestor: HTMLElement | null = control; ancestor; ancestor = ancestor.parentElement) {
    const style = window.getComputedStyle(ancestor);
    if (
      style.display === "none" ||
      style.visibility === "hidden" ||
      style.visibility === "collapse"
    )
      return false;
    if (
      ancestor instanceof HTMLDetailsElement &&
      !ancestor.open &&
      !ancestor.querySelector(":scope > summary")?.contains(control)
    )
      return false;
  }
  return true;
}

export function TranscriptMenu({ id, trigger, focusLast, onClose, children }: TranscriptMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const position = () => {
      const viewport = window.visualViewport;
      const leftEdge = (viewport?.offsetLeft ?? 0) + 8;
      const topEdge = (viewport?.offsetTop ?? 0) + 8;
      const rightEdge = leftEdge + (viewport?.width ?? window.innerWidth) - 16;
      const bottomEdge = topEdge + (viewport?.height ?? window.innerHeight) - 16;
      const anchor = trigger.getBoundingClientRect();
      menu.style.maxWidth = `${rightEdge - leftEdge}px`;
      const width = Math.min(menu.getBoundingClientRect().width || 224, rightEdge - leftEdge);
      const below = Math.max(0, bottomEdge - anchor.bottom - 6);
      const above = Math.max(0, anchor.top - topEdge - 6);
      const naturalHeight = menu.scrollHeight + 2;
      const opensBelow = naturalHeight <= below || below >= above;
      const availableHeight = Math.min(bottomEdge - topEdge, opensBelow ? below : above);
      menu.style.maxHeight = `${availableHeight}px`;
      const height = Math.min(naturalHeight, availableHeight);
      menu.style.left = `${Math.max(leftEdge, Math.min(anchor.right - width, rightEdge - width))}px`;
      menu.style.top = `${Math.max(topEdge, Math.min(opensBelow ? anchor.bottom + 6 : anchor.top - height - 6, bottomEdge - height))}px`;
      // Keep keyboard focus in the menu if a focused audio action expires and disappears.
      if (document.activeElement === document.body) {
        menu
          .querySelector<HTMLButtonElement>("[role=menuitem]:not(:disabled)")
          ?.focus({ preventScroll: true });
      }
    };
    position();
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    window.visualViewport?.addEventListener("resize", position);
    window.visualViewport?.addEventListener("scroll", position);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(position);
    observer?.observe(menu);
    return () => {
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
      window.visualViewport?.removeEventListener("resize", position);
      window.visualViewport?.removeEventListener("scroll", position);
      observer?.disconnect();
    };
  }, [trigger, children]);

  useLayoutEffect(() => {
    const items = menuRef.current?.querySelectorAll<HTMLButtonElement>(
      "[role=menuitem]:not(:disabled)",
    );
    items?.[focusLast ? items.length - 1 : 0]?.focus({ preventScroll: true });
    const dismissOutside = (event: Event) => {
      const target = event.target;
      if (
        target instanceof Node &&
        !menuRef.current?.contains(target) &&
        !trigger.contains(target)
      ) {
        onClose(false);
      }
    };
    const dismissEscape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("pointerdown", dismissOutside);
    document.addEventListener("focusin", dismissOutside);
    document.addEventListener("keydown", dismissEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOutside);
      document.removeEventListener("focusin", dismissOutside);
      document.removeEventListener("keydown", dismissEscape);
    };
  }, [trigger, focusLast, onClose]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const menu = menuRef.current;
    if (!menu) return;
    const items = Array.from(
      menu.querySelectorAll<HTMLButtonElement>("[role=menuitem]:not(:disabled)"),
    );
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    let next: number;
    switch (event.key) {
      case "ArrowDown":
        next = (index + 1) % items.length;
        break;
      case "ArrowUp":
        next = (index - 1 + items.length) % items.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = items.length - 1;
        break;
      case "Tab": {
        // Continue the page's tab order from the trigger, even though the menu lives in a portal.
        const controls = Array.from(
          document.querySelectorAll<HTMLElement>(
            "button, input, select, textarea, a[href], summary, [tabindex]",
          ),
        )
          .filter((control) => !menu.contains(control) && isPageTabStop(control))
          .sort((a, b) => (a.tabIndex || Infinity) - (b.tabIndex || Infinity));
        const current = controls.indexOf(trigger);
        const target =
          controls[(current + (event.shiftKey ? -1 : 1) + controls.length) % controls.length];
        event.preventDefault();
        onClose(false);
        (target ?? trigger).focus();
        return;
      }
      default:
        return;
    }
    event.preventDefault();
    items[next]?.focus();
  };

  return createPortal(
    <div
      id={id}
      ref={menuRef}
      className="transcript-menu"
      role="menu"
      tabIndex={-1}
      aria-label="Transcript actions"
      onKeyDown={onKeyDown}
    >
      {children}
    </div>,
    document.body,
  );
}
