import type { VariantProps } from 'class-variance-authority'
import { cva } from 'class-variance-authority'

export { default as Button } from './Button.vue'

export const buttonVariants = cva(
  'focus-visible:border-ring focus-visible:ring-ring/50 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive dark:aria-invalid:border-destructive/50 rounded-full border border-transparent bg-clip-padding m3-label-large focus-visible:ring-3 aria-invalid:ring-3 active:not-aria-[haspopup]:translate-y-px [&_svg:not([class*=size-])]:size-4 group/button inline-flex shrink-0 items-center justify-center whitespace-nowrap transition-[background-color,border-color,color,box-shadow,transform] duration-100 ease-[cubic-bezier(0.23,1,0.32,1)] outline-none select-none disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0',
  {
    variants: {
      variant: {
        default: 'bg-primary text-primary-foreground shadow-xs [a]:hover:bg-primary/90',
        outline: 'border-outline bg-transparent text-primary hover:bg-primary/10 aria-expanded:bg-primary/10 aria-expanded:text-primary',
        secondary: 'bg-secondary-container text-on-secondary-container hover:bg-secondary-container/90 aria-expanded:bg-secondary-container aria-expanded:text-on-secondary-container',
        ghost: 'text-primary hover:bg-primary/10 hover:text-primary aria-expanded:bg-primary/10 aria-expanded:text-primary',
        destructive: 'bg-error-container text-on-error-container hover:bg-error-container/85 focus-visible:ring-error-container/30',
        link: 'text-primary underline-offset-4 hover:underline',
      },
      size: {
        'default': 'h-9 gap-1.5 rounded-lg px-3.5 has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3',
        'xs': 'h-7 gap-1 rounded-[min(var(--radius-md),10px)] px-2 text-xs in-data-[slot=button-group]:rounded-md has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*=size-])]:size-3',
        'sm': 'h-8 gap-1 rounded-[min(var(--radius-md),12px)] px-3 text-[0.8rem] in-data-[slot=button-group]:rounded-md has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2 [&_svg:not([class*=size-])]:size-3.5',
        'lg': 'h-10 gap-1.5 rounded-lg px-4 has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3',
        'icon': 'size-9',
        'icon-xs': 'size-7 rounded-full in-data-[slot=button-group]:rounded-md [&_svg:not([class*=size-])]:size-3.5',
        'icon-sm': 'size-8 rounded-full in-data-[slot=button-group]:rounded-md',
        'icon-lg': 'size-10',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  },
)
export type ButtonVariants = VariantProps<typeof buttonVariants>
