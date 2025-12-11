import { NextResponse } from 'next/server';
import path from 'path';
import fs from 'fs/promises';

export const runtime = 'nodejs';

export async function GET() {
  // Canonical repo logo lives at repo root as `logo.jpg`.
  // In dev/prod, Next.js runs with `process.cwd()` = `website/`.
  const filePath = path.join(process.cwd(), '..', 'logo.jpg');

  try {
    const file = await fs.readFile(filePath);
    return new NextResponse(file, {
      headers: {
        'Content-Type': 'image/jpeg',
        // Cache reasonably; logo changes are rare but not impossible.
        'Cache-Control': 'public, max-age=3600, stale-while-revalidate=86400',
      },
    });
  } catch {
    return NextResponse.json({ error: 'Logo not found' }, { status: 404 });
  }
}
