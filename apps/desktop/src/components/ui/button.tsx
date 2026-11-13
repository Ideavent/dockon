import type { ButtonHTMLAttributes } from "react";
import { cva } from "class-variance-authority";
import { clsx } from "clsx";
import { twMerge } from "tailwind-merge";
const variants = cva(
  "inline-flex items-center justify-center gap-2 rounded-lg text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-emerald-400 disabled:opacity-40 disabled:cursor-not-allowed",
  {
    variants: {
      variant: {
        primary: "bg-emerald-400 text-slate-950 hover:bg-emerald-300",
        secondary: "bg-slate-800 text-slate-200 hover:bg-slate-700",
      },
    },
    defaultVariants: { variant: "secondary" },
  },
);
export function Button({
  className,
  variant,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary";
}) {
  return (
    <button
      className={twMerge(clsx(variants({ variant }), "px-4 py-2", className))}
      {...props}
    />
  );
}
