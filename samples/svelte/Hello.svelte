<script>
  let name = "World";
  $: greeting = `Hello, ${name}`;
</script>

<p>{greeting}</p>

{#if name}
  <span>Welcome!</span>
{/if}
