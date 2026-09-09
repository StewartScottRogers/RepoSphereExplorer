<template>
  <section class="board" :class="{ 'board--busy': loading }">
    <header class="board__header">
      <h2>{{ title }}</h2>
      <input
        v-model.trim="filter"
        class="board__filter"
        type="search"
        placeholder="Filter tasks"
        :aria-label="`Filter ${title}`"
      />
      <span class="board__count">{{ visibleTasks.length }} / {{ tasks.length }}</span>
    </header>

    <p v-if="loading" class="board__state">Loading…</p>
    <p v-else-if="visibleTasks.length === 0" class="board__state">{{ emptyMessage }}</p>

    <ul v-else class="board__columns">
      <li v-for="column in columns" :key="column.id" class="board__column">
        <h3>{{ column.label }} <small>{{ tasksIn(column.id).length }}</small></h3>

        <transition-group name="card" tag="ul" class="board__cards">
          <li
            v-for="task in tasksIn(column.id)"
            :key="task.id"
            class="card"
            :class="`card--${task.priority}`"
            @click="$emit('select', task)"
          >
            <strong>{{ task.title }}</strong>
            <span class="card__meta">{{ task.assignee ?? 'unassigned' }}</span>
            <button class="card__advance" @click.stop="advance(task)">→</button>
          </li>
        </transition-group>
      </li>
    </ul>

    <footer class="board__footer">
      <slot name="footer" :total="tasks.length">
        <button :disabled="loading" @click="refresh">Refresh</button>
      </slot>
    </footer>
  </section>
</template>

<script>
import { computed, defineComponent, onMounted, ref, watch } from 'vue'

export default defineComponent({
  name: 'TaskBoard',

  props: {
    title: { type: String, default: 'Tasks' },
    tasks: { type: Array, required: true },
    columns: {
      type: Array,
      default: () => [
        { id: 'todo', label: 'To do' },
        { id: 'doing', label: 'In progress' },
        { id: 'done', label: 'Done' },
      ],
    },
    loading: { type: Boolean, default: false },
    emptyMessage: { type: String, default: 'Nothing here yet' },
  },

  emits: ['select', 'move', 'refresh'],

  setup(props, { emit }) {
    const filter = ref('')
    const lastMoved = ref(null)

    const visibleTasks = computed(() => {
      if (!filter.value) {
        return props.tasks
      }
      const needle = filter.value.toLowerCase()
      return props.tasks.filter(
        (task) =>
          task.title.toLowerCase().includes(needle) ||
          (task.assignee ?? '').toLowerCase().includes(needle),
      )
    })

    function tasksIn(columnId) {
      return visibleTasks.value.filter((task) => task.status === columnId)
    }

    function nextColumn(status) {
      const index = props.columns.findIndex((column) => column.id === status)
      return props.columns[Math.min(index + 1, props.columns.length - 1)].id
    }

    function advance(task) {
      const target = nextColumn(task.status)
      if (target !== task.status) {
        lastMoved.value = task.id
        emit('move', { id: task.id, from: task.status, to: target })
      }
    }

    function refresh() {
      emit('refresh')
    }

    watch(
      () => props.tasks.length,
      (now, before) => {
        if (now < before) {
          filter.value = ''
        }
      },
    )

    onMounted(() => {
      if (props.tasks.length === 0) {
        emit('refresh')
      }
    })

    return { filter, lastMoved, visibleTasks, tasksIn, advance, refresh }
  },
})
</script>

<style scoped>
.board {
  display: flex;
  flex-direction: column;
  gap: 12px;
  font-family: 'Segoe UI', system-ui, sans-serif;
}

.board--busy {
  opacity: 0.6;
}

.board__header {
  display: flex;
  align-items: center;
  gap: 12px;
}

.board__columns {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 12px;
  list-style: none;
  padding: 0;
}

.card {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px;
  border: 1px solid #dcdcdc;
  border-radius: 4px;
  cursor: pointer;
}

.card--high {
  border-left: 4px solid #c0392b;
}

.card__meta {
  margin-left: auto;
  color: #6b7280;
  font-size: 0.85em;
}

.card-enter-active,
.card-leave-active {
  transition: opacity 0.15s ease;
}

.card-enter-from,
.card-leave-to {
  opacity: 0;
}
</style>
