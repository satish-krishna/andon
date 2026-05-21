import type { BudgetStatus } from '../../core/api.service';

/** Tailwind text-colour class for a budget status. */
export function budgetTextClass(status: BudgetStatus): string {
  switch (status) {
    case 'red':
      return 'text-err';
    case 'amber':
      return 'text-warn';
    default:
      return 'text-muted';
  }
}

/** Tailwind background-colour class for the budget progress bar. */
export function budgetBarClass(status: BudgetStatus): string {
  switch (status) {
    case 'red':
      return 'bg-err';
    case 'amber':
      return 'bg-warn';
    default:
      return 'bg-muted';
  }
}
