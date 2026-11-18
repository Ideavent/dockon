import type { ButtonHTMLAttributes } from "react";
import { clsx } from "clsx";

export function Button({
  className,
  variant,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary";
}) {
  return (
    <button
      className={clsx(
        "ui-button",
        variant === "primary" && "ui-button-primary",
        className,
      )}
      {...props}
    />
  );
}
