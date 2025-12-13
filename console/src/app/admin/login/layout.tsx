/**
 * Operator Login Layout
 * 
 * This layout bypasses the admin gate since users need to be able to 
 * access the login page without being authenticated as an operator.
 */
export default function OperatorLoginLayout({ children }: { children: React.ReactNode }) {
  return children
}
