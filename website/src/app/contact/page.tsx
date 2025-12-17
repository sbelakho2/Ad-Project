'use client';

import { motion } from 'framer-motion';
import {
  Building2,
  Clock,
  Mail,
  MessageSquare,
  Send,
  ShieldCheck,
} from 'lucide-react';
import { useState } from 'react';

import CookieBanner from '@/components/CookieBanner';
import Footer from '@/components/Footer';
import { ContactBenefitsCard } from '@/components/VisualCard';
import PageHero from '@/components/PageHero';
import SiteHeader from '@/components/SiteHeader';
import { legalEntity } from '@/config/legal';

const getRevealProps = (delay = 0) => ({
  initial: false,
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.5, delay },
});

const contactReasons = [
  'General inquiry',
  'Sales / Pricing',
  'Technical support',
  'Partnership',
  'Security / Compliance',
  'Press / Media',
];

export default function ContactPage() {
  const [formData, setFormData] = useState({
    name: '',
    email: '',
    company: '',
    reason: '',
    message: '',
  });
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isSubmitted, setIsSubmitted] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsSubmitting(true);
    
    // Simulate form submission
    await new Promise(resolve => setTimeout(resolve, 1000));
    
    setIsSubmitting(false);
    setIsSubmitted(true);
  };

  const handleChange = (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>) => {
    setFormData(prev => ({
      ...prev,
      [e.target.name]: e.target.value,
    }));
  };

  const routeHint = (() => {
    switch (formData.reason) {
      case 'Security / Compliance':
        return {
          title: 'Security / Compliance routing',
          body: `For security and compliance inquiries, you can also email ${legalEntity.contact.securityEmail}.`,
        };
      case 'Technical support':
        return {
          title: 'Technical support routing',
          body: `If you are an existing customer, your support channel may include Slack; otherwise submit this form and we will route your request.`,
        };
      default:
        return null;
    }
  })();

  return (
    <>
      <main className="relative bg-white text-slate-900">
        <SiteHeader />

        {/* Hero */}
        <PageHero>
          <div className="text-center">
            <motion.div {...getRevealProps(0)} className="max-w-3xl mx-auto">
              <h1 className="text-4xl md:text-5xl font-bold leading-tight mb-4">
                Let&apos;s talk.
              </h1>
              <p className="text-xl leading-relaxed">
                Whether you&apos;re evaluating, integrating, or scaling—we&apos;re here to help.
              </p>
            </motion.div>
          </div>
        </PageHero>

        {/* Contact Grid */}
        <section className="py-12 pb-20">
          <div className="container mx-auto px-4">
            <div className="max-w-6xl mx-auto">
              <div className="grid lg:grid-cols-5 gap-12">
                {/* Contact Info */}
                <motion.div {...getRevealProps(0)} className="lg:col-span-2 space-y-8">
                  <div>
                    <h2 className="text-2xl font-bold text-gray-900 mb-6">Get in touch</h2>
                    <p className="text-gray-600 mb-8">
                      We typically respond within one business day. 
                      For urgent issues, mention it in your message.
                    </p>
                  </div>

                  <div className="space-y-6">
                    <div className="flex items-start gap-4">
                      <div className="w-12 h-12 bg-brand-100 rounded-xl flex items-center justify-center flex-shrink-0">
                        <Mail className="w-6 h-6 text-brand-600" />
                      </div>
                      <div>
                        <h3 className="font-semibold text-gray-900 mb-1">Email</h3>
                        <a 
                          href={`mailto:${legalEntity.contact.email}`}
                          className="text-brand-600 hover:underline"
                        >
                          {legalEntity.contact.email}
                        </a>
                        <p className="text-xs text-gray-600 mt-1">
                          General inquiries and sales.
                        </p>
                      </div>
                    </div>

                    <div className="flex items-start gap-4">
                      <div className="w-12 h-12 bg-brand-100 rounded-xl flex items-center justify-center flex-shrink-0">
                        <ShieldCheck className="w-6 h-6 text-brand-600" />
                      </div>
                      <div>
                        <h3 className="font-semibold text-gray-900 mb-1">Security / compliance</h3>
                        <a
                          href={`mailto:${legalEntity.contact.securityEmail}`}
                          className="text-brand-600 hover:underline"
                        >
                          {legalEntity.contact.securityEmail}
                        </a>
                        <p className="text-xs text-gray-600 mt-1">
                          Security questionnaires, DPAs, and incident reporting.
                        </p>
                      </div>
                    </div>

                    <div className="flex items-start gap-4">
                      <div className="w-12 h-12 bg-brand-100 rounded-xl flex items-center justify-center flex-shrink-0">
                        <Clock className="w-6 h-6 text-brand-600" />
                      </div>
                      <div>
                        <h3 className="font-semibold text-gray-900 mb-1">Response time</h3>
                        <p className="text-gray-600">Within 1 business day</p>
                      </div>
                    </div>

                    <div className="flex items-start gap-4">
                      <div className="w-12 h-12 bg-brand-100 rounded-xl flex items-center justify-center flex-shrink-0">
                        <Building2 className="w-6 h-6 text-brand-600" />
                      </div>
                      <div>
                        <h3 className="font-semibold text-gray-900 mb-1">Office</h3>
                        <p className="text-gray-600">
                          {legalEntity.name} ({legalEntity.registryLine})<br />
                          {legalEntity.address.street}<br />
                          {legalEntity.address.postalCode} {legalEntity.address.city}, {legalEntity.address.country}
                        </p>
                      </div>
                    </div>
                  </div>

                  <div className="pt-6 border-t border-gray-200">
                    <h3 className="font-semibold text-gray-900 mb-3">How we can help</h3>
                    <ul className="space-y-2 text-gray-600 mb-6">
                      <li className="flex items-center gap-2">
                        <MessageSquare className="w-4 h-4 text-brand-600" />
                        SDK integration & migration
                      </li>
                      <li className="flex items-center gap-2">
                        <MessageSquare className="w-4 h-4 text-brand-600" />
                        CTV / SSAI setup (HLS/DASH, Live/VOD)
                      </li>
                      <li className="flex items-center gap-2">
                        <MessageSquare className="w-4 h-4 text-brand-600" />
                        Player stack (Shaka/hls.js/AVPlayer/ExoPlayer)
                      </li>
                      <li className="flex items-center gap-2">
                        <MessageSquare className="w-4 h-4 text-brand-600" />
                        DRM (Widevine/FairPlay/PlayReady) + origin/CDN setup
                      </li>
                      <li className="flex items-center gap-2">
                        <MessageSquare className="w-4 h-4 text-brand-600" />
                        Transparency verification & audit
                      </li>
                      <li className="flex items-center gap-2">
                        <MessageSquare className="w-4 h-4 text-brand-600" />
                        Performance tuning & SLO guidance
                      </li>
                      <li className="flex items-center gap-2">
                        <MessageSquare className="w-4 h-4 text-brand-600" />
                        Custom enterprise requirements
                      </li>
                    </ul>
                    <ContactBenefitsCard className="mt-4" />
                  </div>
                </motion.div>

                {/* Contact Form */}
                <motion.div {...getRevealProps(0.1)} className="lg:col-span-3">
                  <div className="bg-gray-50 rounded-2xl p-8">
                    {isSubmitted ? (
                      <div className="text-center py-12">
                        <div className="w-16 h-16 bg-green-100 rounded-full flex items-center justify-center mx-auto mb-6">
                          <Send className="w-8 h-8 text-green-600" />
                        </div>
                        <h3 className="text-2xl font-bold text-gray-900 mb-2">Message sent!</h3>
                        <p className="text-gray-600 mb-6">
                          Thanks for reaching out. We&apos;ll get back to you within one business day.
                        </p>
                        <button
                          onClick={() => {
                            setIsSubmitted(false);
                            setFormData({ name: '', email: '', company: '', reason: '', message: '' });
                          }}
                          className="text-brand-600 font-semibold hover:underline"
                        >
                          Send another message
                        </button>
                      </div>
                    ) : (
                      <form onSubmit={handleSubmit} className="space-y-6">
                        <div className="grid md:grid-cols-2 gap-6">
                          <div>
                            <label htmlFor="name" className="block text-sm font-medium text-gray-700 mb-2">
                              Name *
                            </label>
                            <input
                              type="text"
                              id="name"
                              name="name"
                              required
                              value={formData.name}
                              onChange={handleChange}
                              className="w-full px-4 py-3 rounded-lg border border-gray-300 focus:ring-2 focus:ring-brand-500 focus:border-brand-500 transition-colors"
                              placeholder="Your name"
                            />
                          </div>
                          <div>
                            <label htmlFor="email" className="block text-sm font-medium text-gray-700 mb-2">
                              Email *
                            </label>
                            <input
                              type="email"
                              id="email"
                              name="email"
                              required
                              value={formData.email}
                              onChange={handleChange}
                              className="w-full px-4 py-3 rounded-lg border border-gray-300 focus:ring-2 focus:ring-brand-500 focus:border-brand-500 transition-colors"
                              placeholder="you@company.com"
                            />
                          </div>
                        </div>

                        <div className="grid md:grid-cols-2 gap-6">
                          <div>
                            <label htmlFor="company" className="block text-sm font-medium text-gray-700 mb-2">
                              Company
                              <span className="ml-1 text-gray-400 font-normal">(optional)</span>
                            </label>
                            <input
                              type="text"
                              id="company"
                              name="company"
                              value={formData.company}
                              onChange={handleChange}
                              className="w-full px-4 py-3 rounded-lg border border-gray-300 focus:ring-2 focus:ring-brand-500 focus:border-brand-500 transition-colors"
                              placeholder="Your company"
                            />
                            <p className="mt-1.5 text-xs text-gray-500">
                              Helps us understand your use case.
                            </p>
                          </div>
                          <div>
                            <label htmlFor="reason" className="block text-sm font-medium text-gray-700 mb-2">
                              What can we help with? *
                            </label>
                            <select
                              id="reason"
                              name="reason"
                              required
                              value={formData.reason}
                              onChange={handleChange}
                              aria-describedby={routeHint ? 'reason-help' : undefined}
                              className="w-full px-4 py-3 rounded-lg border border-gray-300 focus:ring-2 focus:ring-brand-500 focus:border-brand-500 transition-colors bg-white"
                            >
                              <option value="">Select a topic</option>
                              {contactReasons.map(reason => (
                                <option key={reason} value={reason}>{reason}</option>
                              ))}
                            </select>
                            {routeHint && (
                              <p id="reason-help" className="mt-2 text-sm text-gray-600" role="status" aria-live="polite">
                                <span className="font-medium text-gray-900">{routeHint.title}:</span> {routeHint.body}
                              </p>
                            )}
                          </div>
                        </div>

                        <div>
                          <label htmlFor="message" className="block text-sm font-medium text-gray-700 mb-2">
                            Message *
                          </label>
                          <textarea
                            id="message"
                            name="message"
                            required
                            rows={5}
                            value={formData.message}
                            onChange={handleChange}
                            className="w-full px-4 py-3 rounded-lg border border-gray-300 focus:ring-2 focus:ring-brand-500 focus:border-brand-500 transition-colors resize-none"
                            placeholder="Tell us about your project, questions, or how we can help..."
                          />
                        </div>

                        <button
                          type="submit"
                          disabled={isSubmitting}
                          className="btn-primary w-full disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
                        >
                          {isSubmitting ? (
                            <>
                              <span className="animate-spin">⏳</span>
                              Sending...
                            </>
                          ) : (
                            <>
                              <Send className="w-5 h-5" />
                              Send message
                            </>
                          )}
                        </button>

                        <p className="text-sm text-gray-500 text-center">
                          By submitting this form, you agree to our{' '}
                          <a href="/privacy" className="text-brand-600 hover:underline">Privacy Policy</a>.
                        </p>
                      </form>
                    )}
                  </div>
                </motion.div>
              </div>
            </div>
          </div>
        </section>
      </main>

      <Footer />
      <CookieBanner />
    </>
  );
}
