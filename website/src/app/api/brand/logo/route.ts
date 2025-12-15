import { NextResponse } from 'next/server';
import path from 'path';
import fs from 'fs/promises';

export const runtime = 'nodejs';

export async function GET() {
  // Try multiple paths to find the logo - first public folder, then parent
  const paths = [
    path.join(process.cwd(), 'public', 'Logo.jpg'),
    path.join(process.cwd(), 'public', 'logo.jpg'),
    path.join(process.cwd(), '..', 'logo.jpg'),
  ];

  for (const filePath of paths) {
    try {
      const file = await fs.readFile(filePath);
      return new NextResponse(file, {
        headers: {
          'Content-Type': 'image/jpeg',
          'Cache-Control': 'public, max-age=3600, stale-while-revalidate=86400',
        },
      });
    } catch {
      // Try next path
    }
  }

  return NextResponse.json({ error: 'Logo not found' }, { status: 404 });
}
