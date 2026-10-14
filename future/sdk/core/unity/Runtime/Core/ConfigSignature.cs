using System;
using System.Security.Cryptography;
using System.Text;

namespace Apex.Mediation.Core
{
    internal static class ConfigSignature
    {
        public static string Sign(string payload, string secret)
        {
            if (string.IsNullOrEmpty(secret))
            {
                throw new ArgumentException("Secret must be provided", nameof(secret));
            }

            using var hmac = new HMACSHA256(Encoding.UTF8.GetBytes(secret));
            var hash = hmac.ComputeHash(Encoding.UTF8.GetBytes(payload));
            return Convert.ToBase64String(hash);
        }

        public static bool Verify(string payload, string secret, string signature)
        {
            if (string.IsNullOrEmpty(signature))
            {
                return false;
            }

            try
            {
                var expected = Sign(payload, secret);
                return FixedTimeEquals(Convert.FromBase64String(expected), Convert.FromBase64String(signature));
            }
            catch (FormatException)
            {
                return false;
            }
        }

        private static bool FixedTimeEquals(ReadOnlySpan<byte> left, ReadOnlySpan<byte> right)
        {
            if (left.Length != right.Length)
            {
                return false;
            }

            var diff = 0;
            for (var i = 0; i < left.Length; i++)
            {
                diff |= left[i] ^ right[i];
            }

            return diff == 0;
        }
    }
}
