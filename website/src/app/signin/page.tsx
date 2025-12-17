import { redirect } from 'next/navigation';

export const dynamic = 'force-dynamic';

export default function SignInPage({
  searchParams,
}: {
  searchParams?: { redirect?: string };
}) {
  const url = new URL('https://console.apexmediation.ee/login');
  const redirectTo = searchParams?.redirect;
  if (redirectTo) {
    url.searchParams.set('redirect', redirectTo);
  }
  redirect(url.toString());
}
