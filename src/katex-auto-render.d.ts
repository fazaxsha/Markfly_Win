// Minimal type declarations for katex/contrib/auto-render (avoids the
// @types/katex dependency; only the subset used by the preview is declared).
declare module 'katex/contrib/auto-render' {
  interface DelimiterSpec {
    left: string
    right: string
    display: boolean
  }

  interface RenderOptions {
    delimiters?: DelimiterSpec[]
    ignoredTags?: string[]
    ignoredClasses?: string[]
    throwOnError?: boolean
    macros?: Record<string, string>
    minRuleThickness?: number
  }

  function renderMathInElement(element: HTMLElement, options?: RenderOptions): void

  export default renderMathInElement
}
