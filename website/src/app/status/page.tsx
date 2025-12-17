import StatusClient from './StatusClient'

export const metadata = {
  title: 'Status — ApexMediation',
  description: 'Service status for ApexMediation: website, API, and console.',
}

export default function StatusPage() {
  return (
    <StatusClient />
  )
}
