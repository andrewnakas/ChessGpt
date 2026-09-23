<script lang="ts">
  interface Props {
    /** White's win chance, 0..100. */
    win: number;
    label: string;
    orientation?: 'white' | 'black';
  }
  let { win, label, orientation = 'white' }: Props = $props();
  const white = $derived(Math.max(2, Math.min(98, win)));
</script>

<div class="bar" class:flipped={orientation === 'black'} title="Evaluation {label}" aria-label="Evaluation {label}">
  <div class="white" style="height: {white}%"></div>
  <span class="label" class:top={white < 50}>{label}</span>
</div>

<style>
  .bar {
    position: relative;
    width: 22px;
    height: 100%;
    border-radius: 6px;
    overflow: hidden;
    background: var(--eval-black);
    border: 1px solid var(--border);
    display: flex;
    flex-direction: column-reverse;
  }
  .bar.flipped {
    flex-direction: column;
  }
  .white {
    background: var(--eval-white);
    transition: height 0.35s ease;
  }
  .label {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 3px;
    text-align: center;
    font-size: 10px;
    font-weight: 700;
    color: #333;
    font-family: var(--mono);
  }
  .label.top {
    bottom: auto;
    top: 3px;
    color: #ddd;
  }
  .flipped .label {
    bottom: auto;
    top: 3px;
  }
  .flipped .label.top {
    top: auto;
    bottom: 3px;
  }
</style>
