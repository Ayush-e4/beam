import { render, screen } from '@testing-library/react';

import HomePage from './page';

describe('HomePage', () => {
  it('shows the product name and tagline', () => {
    render(<HomePage />);

    expect(screen.getByRole('heading', { name: 'Beam' })).toBeInTheDocument();
    expect(screen.getByText(/every device, connected/i)).toBeInTheDocument();
  });
});
