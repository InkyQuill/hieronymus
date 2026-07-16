<script lang="ts">
  type Props = {
    message: string;
    tone?: "success" | "error";
    onDismiss: () => void;
  };
  let { message, tone = "success", onDismiss }: Props = $props();
</script>

<aside class:toast-error={tone === "error"} class="toast" role="status">
  <span>{message}</span><button aria-label="Dismiss notification" onclick={onDismiss}>×</button>
</aside>
