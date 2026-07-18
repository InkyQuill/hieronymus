<script lang="ts">
  type Props = { message: string; tone?: "success" | "error"; onDismiss: () => void };
  let { message, tone = "success", onDismiss }: Props = $props();
</script>

<aside class="animate-toast-in fixed right-4 bottom-4 z-50 flex max-w-sm items-center gap-3 rounded-md border px-4 py-3 text-body-sm shadow-lg motion-reduce:animate-none {tone === 'error' ? 'border-danger bg-[var(--hiero-danger-bg)] text-danger' : 'border-success bg-[var(--hiero-success-bg)] text-success'}" role="status" aria-live="polite">
  <span class="min-w-0 flex-1">{message}</span>
  <button class="inline-flex size-11 shrink-0 items-center justify-center rounded-sm border border-current text-2xl leading-none hover:bg-black/5 dark:hover:bg-white/10" aria-label="Dismiss notification" onclick={onDismiss}>&times;</button>
</aside>
