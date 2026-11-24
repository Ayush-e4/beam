import type { NextConfig } from 'next';

const nextConfig: NextConfig = {
  // Static export: the marketing site is served from a CDN with no server runtime.
  output: 'export',
  reactStrictMode: true,
  poweredByHeader: false,
};

export default nextConfig;
