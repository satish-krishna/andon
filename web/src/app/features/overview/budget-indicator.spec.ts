import { budgetTextClass, budgetBarClass } from './budget-indicator';

describe('budget-indicator', () => {
  it('maps status to a text-colour class', () => {
    expect(budgetTextClass('neutral')).toBe('text-muted');
    expect(budgetTextClass('amber')).toBe('text-warn');
    expect(budgetTextClass('red')).toBe('text-err');
  });

  it('maps status to a bar background class', () => {
    expect(budgetBarClass('neutral')).toBe('bg-muted');
    expect(budgetBarClass('amber')).toBe('bg-warn');
    expect(budgetBarClass('red')).toBe('bg-err');
  });
});
