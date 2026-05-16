'use strict';

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

function getInput(name, options = {}) {
  const key = `INPUT_${name.replace(/ /g, '_').toUpperCase()}`;
  const value = (process.env[key] || options.defaultValue || '').trim();
  if (options.required && !value) {
    throw new Error(`Missing required input: ${name}`);
  }
  return value;
}

function setOutput(name, value) {
  if (!process.env.GITHUB_OUTPUT) {
    return;
  }
  fs.appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${String(value)}\n`);
}

function appendStepSummary(text) {
  if (!process.env.GITHUB_STEP_SUMMARY) {
    return;
  }
  fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
}

async function requestJson(url, token) {
  const response = await fetch(url, {
    headers: {
      Accept: 'application/vnd.github+json',
      Authorization: `Bearer ${token}`,
      'X-GitHub-Api-Version': '2022-11-28',
    },
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`GitHub API ${response.status} ${response.statusText}: ${text.slice(0, 500)}`);
  }
  return text ? JSON.parse(text) : null;
}

async function requestOpenRouter(model, apiKey, repository, payload) {
  const response = await fetch('https://openrouter.ai/api/v1/chat/completions', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${apiKey}`,
      'Content-Type': 'application/json',
      'HTTP-Referer': `https://github.com/${repository}`,
      'X-Title': 'player release notes',
    },
    body: JSON.stringify({
      model,
      temperature: 0.2,
      messages: payload,
    }),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`OpenRouter API ${response.status} ${response.statusText}: ${text.slice(0, 500)}`);
  }
  return text ? JSON.parse(text) : null;
}

function parseSemverTag(tagName) {
  const match = /^v(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.-]+)?$/.exec(tagName);
  if (!match) {
    return null;
  }
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
  };
}

function compareSemver(left, right) {
  return (
    left.major - right.major ||
    left.minor - right.minor ||
    left.patch - right.patch
  );
}

async function fetchTags(apiUrl, repository, token) {
  const tags = [];
  for (let page = 1; page <= 10; page += 1) {
    const pageTags = await requestJson(`${apiUrl}/repos/${repository}/tags?per_page=100&page=${page}`, token);
    if (!Array.isArray(pageTags) || pageTags.length === 0) {
      break;
    }
    tags.push(...pageTags);
    if (pageTags.length < 100) {
      break;
    }
  }
  return tags;
}

function findPreviousTag(tags, currentTag) {
  const otherTags = tags
    .map((tag) => tag.name)
    .filter((tagName) => tagName && tagName !== currentTag);

  const currentSemver = parseSemverTag(currentTag);
  if (!currentSemver) {
    return otherTags[0] || '';
  }

  const semverTags = otherTags
    .map((tagName) => ({ tagName, semver: parseSemverTag(tagName) }))
    .filter((entry) => entry.semver)
    .sort((left, right) => compareSemver(right.semver, left.semver));

  const previous = semverTags.find((entry) => compareSemver(entry.semver, currentSemver) < 0);
  return previous ? previous.tagName : semverTags[0]?.tagName || '';
}

function normalizeCommits(commits) {
  return commits
    .map((commit) => {
      const rawMessage = commit?.commit?.message || '';
      const [subjectLine, ...bodyLines] = rawMessage.split('\n');
      return {
        sha: String(commit?.sha || '').slice(0, 7),
        subject: subjectLine.trim(),
        body: bodyLines.join(' ').replace(/\s+/g, ' ').trim().slice(0, 220),
        author: commit?.commit?.author?.name || commit?.author?.login || 'unknown',
      };
    })
    .filter((commit) => commit.subject);
}

function normalizeFiles(files) {
  return (files || []).map((file) => `${file.status}: ${file.filename}`);
}

async function fetchReleaseContext(apiUrl, repository, token, currentTag, previousTag, maxCommits) {
  if (previousTag) {
    const compare = await requestJson(
      `${apiUrl}/repos/${repository}/compare/${encodeURIComponent(previousTag)}...${encodeURIComponent(currentTag)}`,
      token,
    );
    const commits = normalizeCommits(compare.commits || []).slice(-maxCommits);
    return {
      previousTag,
      compareUrl: compare.html_url || `https://github.com/${repository}/compare/${previousTag}...${currentTag}`,
      commits,
      files: normalizeFiles(compare.files).slice(0, 120),
      commitCount: compare.total_commits || commits.length,
    };
  }

  const commits = await requestJson(
    `${apiUrl}/repos/${repository}/commits?sha=${encodeURIComponent(currentTag)}&per_page=${Math.min(maxCommits, 100)}`,
    token,
  );

  return {
    previousTag: '',
    compareUrl: `https://github.com/${repository}/commits/${encodeURIComponent(currentTag)}`,
    commits: normalizeCommits(commits || []).slice(0, maxCommits),
    files: [],
    commitCount: Array.isArray(commits) ? commits.length : 0,
  };
}

function extractModelContent(payload) {
  const content = payload?.choices?.[0]?.message?.content;
  if (Array.isArray(content)) {
    return content
      .map((part) => {
        if (typeof part === 'string') {
          return part;
        }
        return part?.text || '';
      })
      .join('');
  }
  return typeof content === 'string' ? content : '';
}

function extractJson(text) {
  const direct = text.trim();
  if (!direct) {
    throw new Error('Model response was empty');
  }

  try {
    return JSON.parse(direct);
  } catch (_) {}

  const fenced = direct.match(/```(?:json)?\s*([\s\S]*?)```/i);
  if (fenced) {
    return JSON.parse(fenced[1]);
  }

  const start = direct.indexOf('{');
  const end = direct.lastIndexOf('}');
  if (start !== -1 && end !== -1 && end > start) {
    return JSON.parse(direct.slice(start, end + 1));
  }

  throw new Error('Failed to parse JSON from model response');
}

function cleanSentence(value) {
  return String(value || '').replace(/\s+/g, ' ').trim();
}

function normalizeAiNotes(notes, commitCount) {
  const summary = cleanSentence(notes.summary);
  const highlights = Array.isArray(notes.highlights)
    ? notes.highlights.map(cleanSentence).filter(Boolean).slice(0, 5)
    : [];
  const otherImprovement = cleanSentence(notes.other_improvements || notes.otherImprovements || '');

  if (!summary || highlights.length === 0) {
    throw new Error('Model response did not include the required summary and highlights');
  }

  return {
    summary,
    highlights,
    otherImprovement: otherImprovement || (commitCount > highlights.length
      ? 'Other improvements, fixes, and maintenance work are also included in this release.'
      : ''),
  };
}

function cleanCommitSubject(subject) {
  return subject
    .replace(/^[a-z]+(?:\([^)]*\))?!?:\s*/i, '')
    .replace(/^merge pull request.*$/i, '')
    .replace(/^merge branch.*$/i, '')
    .replace(/\s+/g, ' ')
    .trim();
}

function isUserVisibleSubject(subject) {
  const text = subject.toLowerCase();
  const technicalPattern = /(ci|workflow|release|version|changelog|build|deps|dependency|refactor|format|lint|cargo|tauri\.conf|package\.json)/;
  const userPattern = /(play|playback|player|queue|library|playlist|android|notification|download|search|sync|scroll|history|recent|device|cover|album|track|ui|lock)/;
  return userPattern.test(text) && !technicalPattern.test(text);
}

function buildFallbackNotes(context) {
  const highlights = [];
  const seen = new Set();

  for (const commit of context.commits) {
    const subject = cleanCommitSubject(commit.subject);
    if (!subject || seen.has(subject.toLowerCase()) || !isUserVisibleSubject(subject)) {
      continue;
    }
    highlights.push(subject.charAt(0).toUpperCase() + subject.slice(1));
    seen.add(subject.toLowerCase());
    if (highlights.length === 4) {
      break;
    }
  }

  if (highlights.length === 0) {
    highlights.push('This release focuses on stability, polish, and user-facing fixes across the app.');
  }

  return {
    summary: 'This release brings a small set of user-facing fixes and polish updates, with the rest of the work focused on stability and maintenance.',
    highlights,
    otherImprovement: context.commitCount > highlights.length
      ? 'Other improvements, fixes, and maintenance work are also included in this release.'
      : '',
  };
}

function renderNotes(tagName, context, notes) {
  const lines = [notes.summary, '', '## Highlights'];
  for (const highlight of notes.highlights) {
    lines.push(`- ${highlight}`);
  }
  if (notes.otherImprovement) {
    lines.push(`- ${notes.otherImprovement}`);
  }
  if (context.compareUrl) {
    lines.push('');
    if (context.previousTag) {
      lines.push(`Full changelog: [${context.previousTag}...${tagName}](${context.compareUrl})`);
    } else {
      lines.push(`Recent commits: [${tagName}](${context.compareUrl})`);
    }
  }
  return `${lines.join('\n')}\n`;
}

async function generateAiNotes(apiKey, model, repository, tagName, context) {
  const commitList = context.commits
    .map((commit) => {
      const extra = commit.body ? ` — ${commit.body}` : '';
      return `- ${commit.subject}${extra}`;
    })
    .join('\n');

  const fileList = context.files.length > 0
    ? context.files.map((file) => `- ${file}`).join('\n')
    : '- No file summary available';

  const payload = [
    {
      role: 'system',
      content: [
        'You write GitHub release notes for end users of a music player app.',
        'Focus only on what matters to users: visible features, fixes, playback behavior, UI changes, and meaningful quality-of-life improvements.',
        'Internal tooling, CI, refactors, dependency bumps, and maintenance should be collapsed into one short catch-all sentence.',
        'Return strict JSON only with keys: summary, highlights, other_improvements.',
        'summary must be 1-2 short sentences.',
        'highlights must be an array of 2-5 concise bullet strings.',
        'other_improvements must be a single sentence that groups the remaining work.',
        'Do not mention commit hashes, pull requests, files, CI, workflows, version bumps, or implementation details unless directly user-visible.',
      ].join(' '),
    },
    {
      role: 'user',
      content: [
        `Repository: ${repository}`,
        `Current release tag: ${tagName}`,
        `Previous release tag: ${context.previousTag || 'none'}`,
        `Commit count in this release: ${context.commitCount}`,
        '',
        'Commits included in this release:',
        commitList || '- No commit data available',
        '',
        'Changed files summary:',
        fileList,
        '',
        'Write release notes in plain, user-friendly English.',
      ].join('\n'),
    },
  ];

  const response = await requestOpenRouter(model, apiKey, repository, payload);
  const content = extractModelContent(response);
  const parsed = extractJson(content);
  return normalizeAiNotes(parsed, context.commitCount);
}

async function main() {
  const githubToken = getInput('github-token', { required: true });
  const openRouterApiKey = getInput('openrouter-api-key');
  const githubApiUrl = getInput('github-api-url', { defaultValue: 'https://api.github.com' });
  const repository = getInput('repository', { required: true });
  const tagName = getInput('tag-name', { required: true });
  const model = getInput('model', { defaultValue: 'openai/gpt-4.1-mini' });
  const maxCommits = Number(getInput('max-commits', { defaultValue: '80' })) || 80;

  const tags = await fetchTags(githubApiUrl, repository, githubToken);
  const previousTag = findPreviousTag(tags, tagName);
  const context = await fetchReleaseContext(githubApiUrl, repository, githubToken, tagName, previousTag, maxCommits);

  let notes;
  let generationMode = 'fallback';

  if (openRouterApiKey) {
    try {
      notes = await generateAiNotes(openRouterApiKey, model, repository, tagName, context);
      generationMode = 'openrouter';
    } catch (error) {
      console.warn(`OpenRouter generation failed, falling back to heuristic notes: ${error.message}`);
    }
  } else {
    console.warn('OPEN_ROUTER key is missing, falling back to heuristic release notes');
  }

  if (!notes) {
    notes = buildFallbackNotes(context);
  }

  const notesPath = path.join(process.env.RUNNER_TEMP || os.tmpdir(), `release-notes-${Date.now()}.md`);
  fs.writeFileSync(notesPath, renderNotes(tagName, context, notes), 'utf8');

  setOutput('notes_path', notesPath);
  setOutput('previous_tag', context.previousTag || '');
  setOutput('generation_mode', generationMode);

  appendStepSummary([
    '## Release Notes Generation',
    `- Tag: ${tagName}`,
    `- Previous tag: ${context.previousTag || '<none>'}`,
    `- Mode: ${generationMode}`,
    `- Notes file: ${notesPath}`,
    '',
  ].join('\n'));
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});