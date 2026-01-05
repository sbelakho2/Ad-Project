'use client'

import { useEffect, useMemo, useRef, useState } from 'react'
import Link from 'next/link'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { teamApi } from '@/lib/api'
import { ArrowLeft, Users, UserPlus, Mail, MoreVertical, Shield, Code, Banknote, Crown } from 'lucide-react'
import type { TeamMember, TeamInvitation, TeamRoleDefinition } from '@/types'

const ROLE_ICON_MAP = {
  owner: Crown,
  admin: Shield,
  developer: Code,
  finance: Banknote,
}

const FALLBACK_ROLES: TeamRoleDefinition[] = [
  {
    id: 'admin',
    label: 'Admin',
    description: 'Full access to settings, placements, and team management',
  },
  {
    id: 'developer',
    label: 'Developer',
    description: 'Manage placements and view analytics, no billing access',
  },
  {
    id: 'finance',
    label: 'Finance',
    description: 'View revenue, manage billing, limited placement access',
  },
]

export default function TeamSettingsPage() {
  const [isInviting, setIsInviting] = useState(false)
  const [email, setEmail] = useState('')
  const [role, setRole] = useState<TeamInvitation['role']>('developer')
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)
  const [menuOpen, setMenuOpen] = useState<string | null>(null)
  const [pendingRemoval, setPendingRemoval] = useState<TeamMember | null>(null)
  const menuContainerRef = useRef<HTMLDivElement | null>(null)

  const queryClient = useQueryClient()

  const {
    data: roleCatalog,
    isLoading: loadingRoles,
    isError: roleError,
  } = useQuery({
    queryKey: ['team', 'roles'],
    queryFn: async () => {
      const { data } = await teamApi.listRoles()
      return data
    },
    retry: 1,
    staleTime: 60 * 60 * 1000,
  })

  const inviteableRoles = useMemo(() => {
    const roles = Array.isArray(roleCatalog) ? roleCatalog : FALLBACK_ROLES
    return roles.filter((definition) => definition.invitable !== false)
  }, [roleCatalog])

  useEffect(() => {
    if (inviteableRoles.length === 0) return
    if (!inviteableRoles.some((definition) => definition.id === role)) {
      setRole(inviteableRoles[0].id)
    }
  }, [inviteableRoles, role])

  useEffect(() => {
    if (!menuOpen) return
    const handleInteraction = (event: MouseEvent | KeyboardEvent) => {
      if (event instanceof KeyboardEvent && event.key === 'Escape') {
        setMenuOpen(null)
        return
      }
      if (event instanceof MouseEvent) {
        if (menuContainerRef.current && !menuContainerRef.current.contains(event.target as Node)) {
          setMenuOpen(null)
        }
      }
    }
    document.addEventListener('mousedown', handleInteraction)
    document.addEventListener('keydown', handleInteraction)
    return () => {
      document.removeEventListener('mousedown', handleInteraction)
      document.removeEventListener('keydown', handleInteraction)
    }
  }, [menuOpen])

  useEffect(() => {
    if (!menuOpen) {
      menuContainerRef.current = null
    }
  }, [menuOpen])

  const { data: members = [], isLoading } = useQuery({
    queryKey: ['team', 'members'],
    queryFn: async () => {
      const { data } = await teamApi.listMembers()
      return data
    },
  })

  const inviteMutation = useMutation({
    mutationFn: async (invitation: TeamInvitation) => {
      await teamApi.inviteMember(invitation)
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['team', 'members'] })
      setMessage({ type: 'success', text: 'Invitation sent successfully.' })
      setEmail('')
      setIsInviting(false)
    },
    onError: () => {
      setMessage({ type: 'error', text: 'Failed to send invitation. Please try again.' })
    },
  })

  const removeMutation = useMutation({
    mutationFn: async (id: string) => {
      await teamApi.removeMember(id)
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['team', 'members'] })
      setMessage({ type: 'success', text: 'Team member removed.' })
      setMenuOpen(null)
      setPendingRemoval(null)
    },
    onError: () => {
      setMessage({ type: 'error', text: 'Failed to remove team member.' })
    },
  })

  const resendMutation = useMutation({
    mutationFn: async (id: string) => {
      await teamApi.resendInvite(id)
    },
    onSuccess: () => {
      setMessage({ type: 'success', text: 'Invitation resent.' })
      setMenuOpen(null)
    },
    onError: () => {
      setMessage({ type: 'error', text: 'Failed to resend invitation.' })
    },
  })

  const handleInvite = (e: React.FormEvent) => {
    e.preventDefault()
    if (!email.trim()) return
    inviteMutation.mutate({ email: email.trim(), role })
  }

  const handleConfirmRemoval = () => {
    if (!pendingRemoval) return
    removeMutation.mutate(pendingRemoval.id)
  }

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white/90 backdrop-blur border-b">
        <div className="max-w-5xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <Link
            href="/settings"
            className="inline-flex items-center gap-2 text-sm text-gray-600 hover:text-gray-900 mb-4"
          >
            <ArrowLeft className="h-4 w-4" aria-hidden={true} />
            Back to Settings
          </Link>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="h-12 w-12 rounded-lg bg-primary-50 text-primary-600 flex items-center justify-center">
                <Users className="h-6 w-6" aria-hidden={true} />
              </div>
              <div>
                <p className="text-sm font-medium text-primary-600">Collaboration</p>
                <h1 className="text-3xl font-semibold text-gray-900 tracking-tight">Team Access</h1>
                <p className="text-sm text-gray-600 mt-1">
                  Invite teammates and assign roles to collaborate securely.
                </p>
              </div>
            </div>
            <button
              onClick={() => setIsInviting(!isInviting)}
              className="btn btn-primary flex items-center gap-2"
            >
              <UserPlus className="h-4 w-4" aria-hidden={true} />
              Invite Member
            </button>
          </div>
        </div>
      </header>

      <main className="max-w-5xl mx-auto px-4 sm:px-6 lg:px-8 py-8 space-y-6">
        {message && (
          <div
            className={`card flex items-start gap-3 ${
              message.type === 'success'
                ? 'bg-success-50 border-success-200'
                : 'bg-danger-50 border-danger-200'
            }`}
          >
            <p className="text-sm text-gray-700">{message.text}</p>
          </div>
        )}

        {isInviting && (
          <form onSubmit={handleInvite} className="card space-y-4">
            <h2 className="text-lg font-semibold text-gray-900">Invite New Team Member</h2>
            <div>
              <label htmlFor="email" className="label">
                Email Address
              </label>
              <input
                id="email"
                type="email"
                className="input"
                placeholder="teammate@example.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
              />
            </div>
            <fieldset>
              <legend className="label">Role</legend>
              <div className="space-y-2">
                {inviteableRoles.length === 0 ? (
                  <p className="text-sm text-gray-500">No inviteable roles configured in this workspace.</p>
                ) : (
                  inviteableRoles.map((roleDefinition) => (
                    <div key={roleDefinition.id}>
                      <input
                        id={`role-${roleDefinition.id}`}
                        type="radio"
                        name="role"
                        value={roleDefinition.id}
                        checked={role === roleDefinition.id}
                        onChange={() => setRole(roleDefinition.id)}
                        className="sr-only peer"
                      />
                      <label
                        htmlFor={`role-${roleDefinition.id}`}
                        aria-label={`Select ${roleDefinition.label} role`}
                        className={`card cursor-pointer border transition flex items-start gap-3 ${
                          role === roleDefinition.id ? 'border-primary-500 ring-2 ring-primary-100' : ''
                        }`}
                      >
                        <span
                          aria-hidden={true}
                          className={`mt-1 inline-flex h-4 w-4 rounded-full border ${
                            role === roleDefinition.id ? 'border-primary-600 bg-primary-600' : 'border-gray-300'
                          }`}
                        />
                        <span>
                          <p className="text-sm font-semibold text-gray-900">{roleDefinition.label}</p>
                          <p className="text-xs text-gray-600 mt-1">{roleDefinition.description}</p>
                        </span>
                      </label>
                    </div>
                  ))
                )}
              </div>
              {roleError && (
                <p className="text-xs text-warning-600 mt-2">Using fallback roles while live metadata loads.</p>
              )}
            </fieldset>
            <div className="flex items-center gap-3">
              <button
                type="button"
                onClick={() => setIsInviting(false)}
                className="btn btn-outline"
              >
                Cancel
              </button>
              <button
                type="submit"
                className="btn btn-primary flex items-center gap-2"
                disabled={inviteMutation.isPending || inviteableRoles.length === 0}
              >
                <Mail className="h-4 w-4" aria-hidden={true} />
                {inviteMutation.isPending ? 'Sending...' : 'Send Invitation'}
              </button>
            </div>
          </form>
        )}

        <section className="card">
          <h2 className="text-lg font-semibold text-gray-900 mb-4">Team Members ({members.length})</h2>
          {isLoading ? (
            <div className="space-y-3">
              {[1, 2, 3].map((i) => (
                <div key={i} className="flex items-center gap-4 p-4 border rounded-lg animate-pulse">
                  <div className="h-10 w-10 bg-gray-200 rounded-full" />
                  <div className="flex-1 space-y-2">
                    <div className="h-4 bg-gray-200 rounded w-1/3" />
                    <div className="h-3 bg-gray-200 rounded w-1/2" />
                  </div>
                </div>
              ))}
            </div>
          ) : members.length === 0 ? (
            <div className="text-center py-12">
              <Users className="h-12 w-12 text-gray-400 mx-auto mb-3" aria-hidden={true} />
              <p className="text-sm text-gray-600">No team members yet. Invite your first collaborator!</p>
            </div>
          ) : (
            <div className="space-y-2">
                    {members.map((member) => {
                      const RoleIcon = ROLE_ICON_MAP[member.role] || Users
                return (
                  <div
                    key={member.id}
                    className="flex items-center gap-4 p-4 border rounded-lg hover:bg-gray-50 transition"
                  >
                    <div className="h-10 w-10 rounded-full bg-primary-100 text-primary-700 flex items-center justify-center font-semibold text-sm">
                      {member.name?.charAt(0).toUpperCase() || member.email.charAt(0).toUpperCase()}
                    </div>
                    <div className="flex-1">
                      <div className="flex items-center gap-2">
                        <p className="text-sm font-semibold text-gray-900">{member.name || member.email}</p>
                        <span
                          className={`badge ${
                            member.status === 'active'
                              ? 'badge-success'
                              : member.status === 'invited'
                              ? 'badge-warning'
                              : 'badge-neutral'
                          }`}
                        >
                          {member.status}
                        </span>
                      </div>
                      <div className="flex items-center gap-2 text-xs text-gray-600 mt-1">
                        <RoleIcon className="h-3 w-3" aria-hidden={true} />
                        <span className="capitalize">{member.role}</span>
                        {member.email !== member.name && <span>• {member.email}</span>}
                      </div>
                    </div>
                    {member.role !== 'owner' && (
                      <div
                        className="relative"
                        ref={menuOpen === member.id ? menuContainerRef : undefined}
                      >
                        <button
                          onClick={() => setMenuOpen(menuOpen === member.id ? null : member.id)}
                          className="p-2 hover:bg-gray-100 rounded"
                        >
                          <MoreVertical className="h-4 w-4 text-gray-600" aria-hidden={true} />
                        </button>
                        {menuOpen === member.id && (
                          <div className="absolute right-0 mt-2 w-40 bg-white border rounded-lg shadow-lg z-10">
                            {member.status === 'invited' && (
                              <button
                                onClick={() => resendMutation.mutate(member.id)}
                                className="block w-full px-4 py-2 text-left text-sm hover:bg-gray-50"
                              >
                                Resend Invite
                              </button>
                            )}
                            <button
                              onClick={() => {
                                setPendingRemoval(member)
                                setMenuOpen(null)
                              }}
                              className="block w-full px-4 py-2 text-left text-sm text-danger-600 hover:bg-danger-50"
                            >
                              Remove
                            </button>
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          )}
        </section>
      </main>

      {pendingRemoval && (
        <div className="fixed inset-0 bg-black/40 backdrop-blur-sm flex items-center justify-center z-20">
          <div className="card max-w-md w-full">
            <h3 className="text-lg font-semibold text-gray-900">Remove team member?</h3>
            <p className="text-sm text-gray-600 mt-2">
              {pendingRemoval.name || pendingRemoval.email} will immediately lose access to the console. This action cannot be undone.
            </p>
            <div className="mt-6 flex items-center justify-end gap-3">
              <button
                type="button"
                className="btn btn-outline"
                onClick={() => setPendingRemoval(null)}
                disabled={removeMutation.isPending}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn btn-primary bg-danger-600 border-danger-600 hover:bg-danger-700 focus-visible:ring-danger-500"
                onClick={handleConfirmRemoval}
                disabled={removeMutation.isPending}
              >
                {removeMutation.isPending ? 'Removing…' : 'Remove member'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
