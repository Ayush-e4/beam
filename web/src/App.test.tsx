import { render, screen } from '@testing-library/react';

import { App } from './App';

describe('App', () => {
  it('shows the product name and connection hint', () => {
    render(<App />);

    expect(screen.getByRole('heading', { name: 'Beam' })).toBeInTheDocument();
    expect(screen.getByText(/scan the code to connect/i)).toBeInTheDocument();
  });
});
