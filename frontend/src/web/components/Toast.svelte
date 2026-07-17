<script lang="ts">
  type Props = {
    message: string;
    tone?: "success" | "error";
    onDismiss: () => void;
  };
  let { message, tone = "success", onDismiss }: Props = $props();
</script>

<aside class:toast-error={tone === "error"} class="toast" role="status" aria-live="polite">
  <span class="toast-msg">{message}</span>
  <button class="btn-icon" aria-label="Dismiss notification" onclick={onDismiss}>&times;</button>
</aside>
